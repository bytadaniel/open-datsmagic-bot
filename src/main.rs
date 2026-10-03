mod console;
mod http;
mod model;
mod planner;

use model::{Command, Desert};
use planner::{Bot, CarpetTelemetry, DoomPolicy, MovementStrategy, Strategy};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TICK_SECONDS: f64 = 0.2;

fn main() {
    if let Err(error) = run() {
        eprintln!("player_2 stopped: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let strategy = match env::var("DATS_PLAYER_STRATEGY") {
        Ok(value) => Strategy::parse(&value)?,
        Err(_) => Strategy::StableProfit,
    };
    let movement_strategy = env::var("DATS_MOVEMENT_STRATEGY")
        .ok()
        .map(|value| MovementStrategy::parse(&value))
        .transpose()?
        .unwrap_or_default();
    let doom_policy = env::var("DATS_DOOM_POLICY")
        .ok()
        .map(|value| DoomPolicy::parse(&value))
        .transpose()?
        .unwrap_or_default();
    let base_url = env::var("DATS_SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let token_file = env::var("DATS_PLAYER_TOKEN_FILE").unwrap_or_else(|_| "token.txt".into());
    let token = resolve_token(env::var("DATS_PLAYER_TOKEN").ok(), &token_file)?;
    let telemetry_file = telemetry_path(&token);

    let horizon = env::var("DATS_PLAN_HORIZON_SECONDS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= TICK_SECONDS)
        .unwrap_or_else(|| strategy.default_horizon());
    let horizon = if strategy == Strategy::AgileTop1 {
        horizon.min(15.0)
    } else {
        horizon
    };
    let manual_control_file = manual_control_path(&token);
    let mut bot = Bot::with_strategies(horizon, strategy, movement_strategy, doom_policy);
    let mut pending = json!({ "transports": [] });
    let mut consecutive_errors = 0_u32;
    let dashboard_label = format!(
        "aim={} movement={} doom={}",
        strategy.label(),
        movement_strategy.label(),
        doom_policy.label()
    );
    let mut dashboard = console::Dashboard::new(dashboard_label);
    let mut tick = 0_u64;
    let mut last_carpets: Vec<CarpetTelemetry> = Vec::new();
    let mut dashboard_status: String;

    loop {
        if !active_manual_carpet_ids(&manual_control_file, unix_time_ms()).is_empty() {
            // The visualizer owns the shared token while its short lease is live.
            // Drop stale commands and do not compete for the API's per-token tick.
            pending = json!({ "transports": [] });
            thread::sleep(Duration::from_secs_f64(TICK_SECONDS));
            continue;
        }
        let started = Instant::now();
        let response = http::post_desert(&base_url, &token, &pending);
        let elapsed = started.elapsed();

        match response {
            Ok((429, _)) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                let backoff = Duration::from_secs_f64(TICK_SECONDS * 1.15);
                dashboard_status = format!(
                    "HTTP 429; retry in {:.0} ms",
                    backoff.as_secs_f64() * 1000.0
                );
                dashboard.render(
                    tick,
                    elapsed,
                    Duration::ZERO,
                    started.elapsed(),
                    Duration::from_secs_f64(bot.rtt()),
                    horizon,
                    0,
                    0.0,
                    0.0,
                    0,
                    0,
                    &last_carpets,
                    &dashboard_status,
                );
                thread::sleep(backoff);
                continue;
            }
            Ok((status, body)) if (200..300).contains(&status) => {
                consecutive_errors = 0;
                let response_json = match serde_json::from_str::<Value>(&body) {
                    Ok(value) => value,
                    Err(error) => {
                        dashboard_status = format!("invalid Desert JSON: {error}");
                        dashboard.render(
                            tick,
                            elapsed,
                            Duration::ZERO,
                            started.elapsed(),
                            Duration::from_secs_f64(bot.rtt()),
                            horizon,
                            0,
                            0.0,
                            0.0,
                            0,
                            0,
                            &last_carpets,
                            &dashboard_status,
                        );
                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                };
                let desert = match Desert::parse(response_json) {
                    Ok(value) => value,
                    Err(error) => {
                        dashboard_status = format!("invalid Desert response: {error}");
                        dashboard.render(
                            tick,
                            elapsed,
                            Duration::ZERO,
                            started.elapsed(),
                            Duration::from_secs_f64(bot.rtt()),
                            horizon,
                            0,
                            0.0,
                            0.0,
                            0,
                            0,
                            &last_carpets,
                            &dashboard_status,
                        );
                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                };
                bot.observe_rtt(elapsed.as_secs_f64());
                let manual_ids = active_manual_carpet_ids(&manual_control_file, unix_time_ms());
                dashboard_status = if !manual_ids.is_empty() {
                    format!("manual control active: {}", manual_ids.join(", "))
                } else if desert.errors.is_empty() {
                    "connected".into()
                } else {
                    format!("server: {}", desert.errors.join("; "))
                };
                let planning_started = Instant::now();
                let plan = bot.plan(&desert);
                let planning = planning_started.elapsed();
                let trajectory_evaluations = plan.trajectory_evaluations;
                let cycle = started.elapsed();
                last_carpets = plan.carpets;
                tick += 1;
                write_telemetry(
                    &telemetry_file,
                    tick,
                    strategy.label(),
                    movement_strategy.label(),
                    doom_policy.label(),
                    horizon,
                    planning.as_secs_f64() * 1000.0,
                    cycle.as_secs_f64() * 1000.0,
                    bot.rtt() * 1000.0,
                    trajectory_evaluations,
                    &last_carpets,
                );
                dashboard.render(
                    tick,
                    elapsed,
                    planning,
                    cycle,
                    Duration::from_secs_f64(bot.rtt()),
                    horizon,
                    trajectory_evaluations,
                    desert.max_speed,
                    desert.max_accel,
                    desert.bounties.len(),
                    desert.anomalies.len(),
                    &last_carpets,
                    &dashboard_status,
                );
                let manual_ids = active_manual_carpet_ids(&manual_control_file, unix_time_ms());
                let commands = plan
                    .commands
                    .into_iter()
                    .filter(|command| !manual_ids.contains(&command.id))
                    .collect();
                pending = command_payload(commands);
                if started.elapsed() < Duration::from_secs_f64(TICK_SECONDS) {
                    thread::sleep(Duration::from_secs_f64(TICK_SECONDS) - started.elapsed());
                }
            }
            Ok((status, body)) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                let status = format!("HTTP {status}: {body}");
                dashboard.render(
                    tick,
                    elapsed,
                    Duration::ZERO,
                    started.elapsed(),
                    Duration::from_secs_f64(bot.rtt()),
                    horizon,
                    0,
                    0.0,
                    0.0,
                    0,
                    0,
                    &last_carpets,
                    &status,
                );
                retry_pause(consecutive_errors);
            }
            Err(error) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                dashboard_status = format!("API request failed: {error}");
                dashboard.render(
                    tick,
                    elapsed,
                    Duration::ZERO,
                    started.elapsed(),
                    Duration::from_secs_f64(bot.rtt()),
                    horizon,
                    0,
                    0.0,
                    0.0,
                    0,
                    0,
                    &last_carpets,
                    &dashboard_status,
                );
                retry_pause(consecutive_errors);
            }
        }
    }
}

fn resolve_token(env_token: Option<String>, token_file: &str) -> Result<String, String> {
    let token = match env_token {
        Some(token) => token.trim().to_string(),
        None => fs::read_to_string(token_file)
            .map_err(|e| format!("cannot read token file {token_file}: {e}"))?
            .trim()
            .to_string(),
    };
    if token.is_empty() || token.contains('\r') || token.contains('\n') {
        return Err("token must be non-empty and cannot contain newlines".into());
    }
    Ok(token)
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn manual_control_path(token: &str) -> PathBuf {
    if let Ok(path) = env::var("DATS_MANUAL_CONTROL_FILE") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("manual_control_{:016x}.json", token_hash(token)))
}

fn telemetry_path(token: &str) -> PathBuf {
    if let Ok(path) = env::var("DATS_PLAYER_TELEMETRY_FILE") {
        return PathBuf::from(path);
    }
    env::temp_dir().join(format!(
        "datsmagic_player2_telemetry_{:016x}.json",
        token_hash(token)
    ))
}

#[allow(clippy::too_many_arguments)]
fn write_telemetry(
    path: &Path,
    tick: u64,
    aim_strategy: &str,
    movement_strategy: &str,
    doom_policy: &str,
    horizon_seconds: f64,
    plan_ms: f64,
    cycle_ms: f64,
    rtt_ms: f64,
    trajectory_evaluations: usize,
    carpets: &[CarpetTelemetry],
) {
    let vector = |v: crate::model::Vec2| json!({"x": v.x, "y": v.y});
    let payload = json!({
        "updatedAtUnixMs": unix_time_ms(),
        "tick": tick,
        "aimStrategy": aim_strategy,
        "movementStrategy": movement_strategy,
        "doomPolicy": doom_policy,
        "horizonSeconds": horizon_seconds,
        "planMs": plan_ms,
        "cycleMs": cycle_ms,
        "rttMs": rtt_ms,
        "trajectoryEvaluations": trajectory_evaluations,
        "carpets": carpets.iter().map(|carpet| json!({
            "id": carpet.id,
            "alive": carpet.alive,
            "position": vector(carpet.position),
            "velocity": vector(carpet.velocity),
            "speed": carpet.speed,
            "currentAcceleration": vector(carpet.current_acceleration),
            "anomalyAcceleration": vector(carpet.anomaly_acceleration),
            "commandAcceleration": carpet.command_acceleration.map(vector),
            "goal": carpet.goal,
            "target": carpet.target.map(vector),
            "targetPoints": carpet.target_points,
            "targetDistance": carpet.target_distance,
            "totalPotentialScore": carpet.total_potential_score,
            "routeScore": carpet.route_score,
            "timeToLastBounty": carpet.time_to_last_bounty,
            "scoreRate": carpet.score_rate,
            "firstBountyRate": carpet.first_bounty_rate,
            "timeToReachScore": carpet.time_to_reach_score,
            "bountyCount": carpet.bounty_count,
            "deathAt": carpet.death_at,
            "deathReason": carpet.death_reason,
            "riskExposure": carpet.risk_exposure,
            "movementDecision": carpet.movement_decision,
        })).collect::<Vec<_>>(),
    });
    let Ok(bytes) = serde_json::to_vec(&payload) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let temporary = path.with_extension("json.tmp");
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(temporary, path);
    }
}

fn token_hash(token: &str) -> u64 {
    token
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn active_manual_carpet_ids(path: &Path, now_ms: u64) -> Vec<String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&contents) else {
        return Vec::new();
    };
    let Some(id) = value.get("carpetId").and_then(Value::as_str) else {
        return Vec::new();
    };
    let Some(expires_at) = value.get("expiresAtUnixMs").and_then(Value::as_u64) else {
        return Vec::new();
    };
    if id.is_empty() || expires_at <= now_ms {
        Vec::new()
    } else {
        vec![id.to_owned()]
    }
}

fn command_payload(commands: Vec<Command>) -> Value {
    let transports: Vec<_> = commands
        .into_iter()
        .map(|command| {
            json!({
                "id": command.id,
                "acceleration": { "x": command.acceleration.x, "y": command.acceleration.y }
            })
        })
        .collect();
    json!({ "transports": transports })
}

fn retry_pause(errors: u32) {
    let exponent = errors.min(5);
    let millis = (250_u64.saturating_mul(1_u64 << exponent)).min(8_000);
    thread::sleep(Duration::from_millis(millis));
}

#[cfg(test)]
mod tests {
    use super::{active_manual_carpet_ids, command_payload, resolve_token, token_hash};
    use crate::model::{Command, Vec2};

    #[test]
    fn emits_canonical_desert_batch_shape() {
        let payload = command_payload(vec![Command {
            id: "player_2_0".into(),
            acceleration: Vec2 { x: 40.0, y: 0.0 },
        }]);
        assert_eq!(payload["transports"][0]["id"], "player_2_0");
        assert_eq!(payload["transports"][0]["acceleration"]["x"], 40.0);
    }

    #[test]
    fn explicit_environment_token_wins_over_token_file() {
        assert_eq!(
            resolve_token(Some("none_top1".into()), "/does/not/exist").unwrap(),
            "none_top1"
        );
        assert!(resolve_token(Some("   ".into()), "token.txt").is_err());
    }

    #[test]
    fn manual_control_lease_expires_and_returns_owned_carpet() {
        let file = std::env::temp_dir().join(format!(
            "datsmagic-manual-control-{}.json",
            std::process::id()
        ));
        std::fs::write(&file, r#"{"carpetId":"player_2_3","expiresAtUnixMs":2000}"#).unwrap();
        assert_eq!(active_manual_carpet_ids(&file, 1000), vec!["player_2_3"]);
        assert!(active_manual_carpet_ids(&file, 2000).is_empty());
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn manual_control_token_hash_is_stable_and_does_not_expose_token() {
        assert_eq!(token_hash("player_2"), token_hash("player_2"));
        assert_eq!(token_hash("player_2"), 0x20252df859219fad);
        assert_ne!(token_hash("player_2"), token_hash("player_1"));
    }
}

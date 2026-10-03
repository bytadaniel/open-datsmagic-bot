use crate::planner::CarpetTelemetry;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

pub struct Dashboard {
    strategy: String,
    interactive: bool,
    last_plain: Instant,
    plan_ewma_ms: Option<f64>,
    max_plan_ms: f64,
    deadline_misses: u64,
}

impl Dashboard {
    pub fn new(strategy: String) -> Self {
        Self {
            strategy,
            interactive: io::stdout().is_terminal(),
            last_plain: Instant::now() - Duration::from_secs(5),
            plan_ewma_ms: None,
            max_plan_ms: 0.0,
            deadline_misses: 0,
        }
    }

    pub fn render(
        &mut self,
        tick: u64,
        elapsed: Duration,
        planning: Duration,
        cycle: Duration,
        rtt: Duration,
        horizon: f64,
        trajectory_evaluations: usize,
        max_speed: f64,
        max_accel: f64,
        bounty_count: usize,
        anomaly_count: usize,
        carpets: &[CarpetTelemetry],
        status: &str,
    ) {
        let alive = carpets.iter().filter(|carpet| carpet.alive).count();
        let plan_ms = planning.as_secs_f64() * 1000.0;
        let cycle_ms = cycle.as_secs_f64() * 1000.0;
        if planning > Duration::ZERO {
            self.plan_ewma_ms = Some(
                self.plan_ewma_ms
                    .map_or(plan_ms, |avg| 0.8 * avg + 0.2 * plan_ms),
            );
            self.max_plan_ms = self.max_plan_ms.max(plan_ms);
            if cycle > Duration::from_millis(200) {
                self.deadline_misses = self.deadline_misses.saturating_add(1);
            }
        }
        let plan_percent = plan_ms / 200.0 * 100.0;
        let slack_ms = 200.0 - cycle_ms;
        if !self.interactive && self.last_plain.elapsed() < Duration::from_secs(5) {
            return;
        }
        let estimated_steps = trajectory_evaluations as f64 * ((horizon + 0.4) / 0.2).ceil();
        let mut out = io::stdout().lock();
        if self.interactive {
            let _ = write!(out, "\x1b[H\x1b[2J");
        }
        let _ = writeln!(out, "player_2 [{strategy}] tick={tick} | HTTP {:.0}ms plan {:.1}ms ({plan_percent:.0}%/200) RTT {:.0}ms | alive {alive}/{} | coins {bounty_count} anomalies {anomaly_count} | horizon {horizon:.0}s | limits V{max_speed:.0} A{max_accel:.0}", elapsed.as_secs_f64()*1000.0, plan_ms, rtt.as_secs_f64()*1000.0, carpets.len(), strategy = self.strategy);
        let _ = writeln!(out, "budget: cycle {cycle_ms:.1}/200ms slack {slack_ms:+.1}ms | plan EWMA {:.1}ms max {:.1}ms | deadline misses {} | routes {} (~{estimated_steps:.0} forecast steps)", self.plan_ewma_ms.unwrap_or(0.0), self.max_plan_ms, self.deadline_misses, trajectory_evaluations);
        let _ = writeln!(out, "status: {status}");
        let _ = out.flush();
        self.last_plain = Instant::now();
    }
}

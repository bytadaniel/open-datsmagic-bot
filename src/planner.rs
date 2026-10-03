use crate::model::{Anomaly, Bounty, Command, Desert, Transport, Vec2};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

const TICK: f64 = 0.2;
const FRICTION: f64 = 0.98;
const BOUNTY_AIM_BUFFER: f64 = 18.0;
const BOUNTY_NEAREST_SHORTLIST: usize = 48;
const BOUNTY_VALUABLE_SHORTLIST: usize = 16;
const BOUNTY_SHORTLIST: usize = BOUNTY_NEAREST_SHORTLIST + BOUNTY_VALUABLE_SHORTLIST;
const SCAN_ANGLE_STEP_DEG: f64 = 5.0;
const AGILE_SCAN_ANGLE_STEP_DEG: f64 = 1.0;
const TACTICAL_ANGLE_STEP_DEG: f64 = 10.0;
const TACTICAL_HORIZON_SECONDS: f64 = 5.0;
const REFINE_HALF_WIDTH_DEG: f64 = 3.0;
const REFINE_ANGLE_STEP_DEG: f64 = 0.25;
const MAX_REFINEMENT_SEEDS: usize = 3;
const TARGET_TIME_BONUS: f64 = 5.0;
const TARGET_TIME_SCALE_SECONDS: f64 = 15.0;
const COURSE_CHANGE_MIN_COST: f64 = 12.0;
const COURSE_CHANGE_SCORE_RATIO: f64 = 0.2;
const COURSE_CHANGE_FULL_COST_DEGREES: f64 = 30.0;
const SPEED_TIE_EPSILON: f64 = 0.015;
const PROFIT_SWITCH_RELATIVE_GAIN: f64 = 0.50;
const PROFIT_SWITCH_MIN_GAIN_PER_SECOND: f64 = 5.0;
const MOVEMENT_SAFETY_HORIZON_SECONDS: f64 = 30.0;
const MOVEMENT_SCAN_STEP_DEG: f64 = 5.0;
const MOVEMENT_ESCAPE_IMMINENT_SECONDS: f64 = 3.0;
const MOVEMENT_SCARCE_SAFE_ROUTE_PERCENT: usize = 10;
const MOVEMENT_RISK_GRID_STEP: f64 = 350.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strategy {
    #[default]
    StableProfit,
    AgileTop1,
}

impl Strategy {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "stable-profit" => Ok(Self::StableProfit),
            "agile-top1" => Ok(Self::AgileTop1),
            other => Err(format!(
                "unknown player strategy '{other}'; expected 'stable-profit' or 'agile-top1'"
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::StableProfit => "stable-profit",
            Self::AgileTop1 => "agile-top1",
        }
    }

    pub fn default_horizon(self) -> f64 {
        match self {
            Self::StableProfit => 30.0,
            Self::AgileTop1 => 15.0,
        }
    }

    fn limit_horizon(self, horizon: f64) -> f64 {
        match self {
            Self::StableProfit => horizon.max(TICK),
            Self::AgileTop1 => horizon.clamp(TICK, 15.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MovementStrategy {
    #[default]
    None,
    Survival,
}

impl MovementStrategy {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "none" => Ok(Self::None),
            "survival" => Ok(Self::Survival),
            other => Err(format!(
                "unknown movement strategy '{other}'; expected 'none' or 'survival'"
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Survival => "survival",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DoomPolicy {
    #[default]
    Collect,
    Fastest,
}

impl DoomPolicy {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "collect" => Ok(Self::Collect),
            "fastest" => Ok(Self::Fastest),
            other => Err(format!(
                "unknown doom policy '{other}'; expected 'collect' or 'fastest'"
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Collect => "collect",
            Self::Fastest => "fastest",
        }
    }
}

#[derive(Clone, Debug, Default)]
struct PlanMemo {
    angle: Option<f64>,
    target: Option<Bounty>,
    target_eta: Option<f64>,
    stalled_ticks: u8,
    resume_angle: Option<f64>,
    resume_target: Option<Bounty>,
}

struct BountyGrid {
    cell_size: f64,
    max_capture_radius: f64,
    cells: HashMap<(i32, i32), Vec<usize>>,
}

impl BountyGrid {
    fn new(bounties: &[Bounty], transport_radius: f64) -> Self {
        let max_capture_radius = bounties
            .iter()
            .map(|bounty| transport_radius.max(0.0) + bounty.radius.max(0.0))
            .fold(transport_radius.max(0.0), f64::max);
        let cell_size = (max_capture_radius * 4.0).max(64.0);
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (index, bounty) in bounties.iter().enumerate() {
            let key = (
                (bounty.position.x / cell_size).floor() as i32,
                (bounty.position.y / cell_size).floor() as i32,
            );
            cells.entry(key).or_default().push(index);
        }
        Self {
            cell_size,
            max_capture_radius,
            cells,
        }
    }

    fn visit_segment(&self, start: Vec2, end: Vec2, mut visit: impl FnMut(usize)) {
        let min_x =
            ((start.x.min(end.x) - self.max_capture_radius) / self.cell_size).floor() as i32;
        let max_x =
            ((start.x.max(end.x) + self.max_capture_radius) / self.cell_size).floor() as i32;
        let min_y =
            ((start.y.min(end.y) - self.max_capture_radius) / self.cell_size).floor() as i32;
        let max_y =
            ((start.y.max(end.y) + self.max_capture_radius) / self.cell_size).floor() as i32;
        for cell_x in min_x..=max_x {
            for cell_y in min_y..=max_y {
                if let Some(indices) = self.cells.get(&(cell_x, cell_y)) {
                    for &index in indices {
                        visit(index);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Goal {
    CollectBounties,
    AvoidDeath,
    FastestDeath,
}

impl Goal {
    fn label(self) -> &'static str {
        match self {
            Self::CollectBounties => "collect-bounties",
            Self::AvoidDeath => "avoid-death",
            Self::FastestDeath => "fastest-death",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeathReason {
    Anomaly,
    Boundary,
    Transport,
}

#[derive(Clone, Debug)]
struct Evaluation {
    angle: f64,
    goal: Goal,
    total_potential_score: f64,
    route_score: f64,
    time_to_last_bounty: Option<f64>,
    score_rate: f64,
    first_bounty_rate: f64,
    time_to_reach_score: f64,
    selection_utility: f64,
    average_speed_ratio: f64,
    terminal_speed_ratio: f64,
    bounty_count: usize,
    continuity: f64,
    keeps_target: bool,
    target: Option<Bounty>,
    death_at: Option<f64>,
    death_reason: Option<DeathReason>,
    risk_exposure: f64,
}

pub struct Bot {
    horizon: f64,
    strategy: Strategy,
    movement_strategy: MovementStrategy,
    doom_policy: DoomPolicy,
    rtt_ewma: f64,
    plans: HashMap<String, PlanMemo>,
}

pub struct PlanResult {
    pub commands: Vec<Command>,
    pub carpets: Vec<CarpetTelemetry>,
    pub trajectory_evaluations: usize,
}

#[derive(Clone)]
pub struct CarpetTelemetry {
    pub id: String,
    pub alive: bool,
    pub position: Vec2,
    pub velocity: Vec2,
    pub speed: f64,
    pub current_acceleration: Vec2,
    pub anomaly_acceleration: Vec2,
    pub command_acceleration: Option<Vec2>,
    pub goal: Option<&'static str>,
    pub target: Option<Vec2>,
    pub target_points: Option<f64>,
    pub target_distance: Option<f64>,
    pub total_potential_score: Option<f64>,
    pub route_score: Option<f64>,
    pub time_to_last_bounty: Option<f64>,
    pub score_rate: Option<f64>,
    pub first_bounty_rate: Option<f64>,
    pub time_to_reach_score: Option<f64>,
    pub bounty_count: usize,
    pub death_at: Option<f64>,
    pub death_reason: Option<&'static str>,
    pub risk_exposure: Option<f64>,
    pub movement_decision: Option<&'static str>,
}

impl Bot {
    pub fn new(horizon: f64) -> Self {
        Self::with_strategy(horizon, Strategy::StableProfit)
    }

    pub fn with_strategy(horizon: f64, strategy: Strategy) -> Self {
        Self::with_strategies(
            horizon,
            strategy,
            MovementStrategy::None,
            DoomPolicy::Collect,
        )
    }

    pub fn with_strategies(
        horizon: f64,
        strategy: Strategy,
        movement_strategy: MovementStrategy,
        doom_policy: DoomPolicy,
    ) -> Self {
        Self {
            horizon: strategy.limit_horizon(horizon),
            strategy,
            movement_strategy,
            doom_policy,
            rtt_ewma: 0.08,
            plans: HashMap::new(),
        }
    }

    pub fn observe_rtt(&mut self, sample: f64) {
        if sample.is_finite() && sample >= 0.0 {
            let bounded = sample.clamp(0.005, 2.0);
            self.rtt_ewma = 0.8 * self.rtt_ewma + 0.2 * bounded;
        }
    }

    pub fn rtt(&self) -> f64 {
        self.rtt_ewma
    }

    pub fn plan(&mut self, desert: &Desert) -> PlanResult {
        if self.strategy == Strategy::AgileTop1 {
            // This mode is explicitly stateless with respect to trajectory choice.
            self.plans.clear();
        }
        let mut transports = desert
            .transports
            .iter()
            .filter(|t| t.alive)
            .collect::<Vec<_>>();
        transports.sort_by(|a, b| a.id.cmp(&b.id));
        let mut commands = Vec::with_capacity(transports.len());
        let mut trajectory_evaluations = 0;
        let mut carpets = desert
            .transports
            .iter()
            .map(|transport| CarpetTelemetry {
                id: transport.id.clone(),
                alive: transport.alive,
                position: transport.position,
                velocity: transport.velocity,
                speed: transport.velocity.length(),
                current_acceleration: transport.self_acceleration,
                anomaly_acceleration: transport.anomaly_acceleration,
                command_acceleration: None,
                goal: None,
                target: None,
                target_points: None,
                target_distance: None,
                total_potential_score: None,
                route_score: None,
                time_to_last_bounty: None,
                score_rate: None,
                time_to_reach_score: None,
                first_bounty_rate: None,
                bounty_count: 0,
                death_at: None,
                death_reason: None,
                risk_exposure: None,
                movement_decision: None,
            })
            .collect::<Vec<_>>();
        let mut reserved_targets: Vec<Vec2> = Vec::new();

        for transport in transports {
            let mut carpet_evaluations = 0;
            let memo = if self.strategy == Strategy::StableProfit {
                self.plans.get(&transport.id).cloned().unwrap_or_default()
            } else {
                PlanMemo::default()
            };
            let mut bounties = if self.strategy == Strategy::AgileTop1 {
                desert.bounties.clone()
            } else {
                shortlist_bounties(transport.position, &desert.bounties, &reserved_targets)
            };
            if let Some(target) = memo
                .target
                .as_ref()
                .filter(|target| !is_reserved(target.position, &reserved_targets))
                .filter(|target| contains_bounty(&desert.bounties, target))
                .filter(|target| !contains_bounty(&bounties, target))
            {
                bounties.push(target.clone());
            }
            if let Some(target) = memo
                .resume_target
                .as_ref()
                .filter(|target| !is_reserved(target.position, &reserved_targets))
                .filter(|target| contains_bounty(&desert.bounties, target))
                .filter(|target| !contains_bounty(&bounties, target))
            {
                bounties.push(target.clone());
            }
            let available_target = memo
                .target
                .as_ref()
                .filter(|target| contains_bounty(&bounties, target));
            let bounty_grid = (self.strategy == Strategy::AgileTop1)
                .then(|| BountyGrid::new(&bounties, desert.transport_radius));
            let mut stalled_ticks = memo.stalled_ticks;
            // The next command is sent on the next polling slot, then waits for the
            // server tick boundary. Include both cadence/phase and observed network RTT.
            let delay = if self.strategy == Strategy::AgileTop1 {
                0.3
            } else {
                (self.rtt_ewma + TICK * 0.5).clamp(TICK * 1.5, 0.8)
            };
            // Keep the first actually collectible bounty on the committed route
            // until it is collected, disappears, or the route becomes unsafe.
            let commitment_horizon = self.horizon.max(TICK);
            let mut held_course = None;
            let held_target = available_target.filter(|target| {
                let Some(angle) = memo.angle else {
                    return false;
                };
                carpet_evaluations += 1;
                let previous_path = evaluate(
                    desert,
                    transport,
                    angle,
                    delay,
                    commitment_horizon,
                    &bounties,
                    &reserved_targets,
                    Some(target),
                    bounty_grid.as_ref(),
                    self.strategy == Strategy::StableProfit,
                    false,
                );
                if let Some(previous_eta) = memo.target_eta {
                    let expected_eta = (previous_eta - TICK).max(0.0);
                    if previous_path.time_to_reach_score > expected_eta + 0.12 {
                        stalled_ticks = stalled_ticks.saturating_add(1);
                    } else {
                        stalled_ticks = 0;
                    }
                }
                let remains_locked = previous_path.death_at.is_none()
                    && previous_path.keeps_target
                    && stalled_ticks < 2;
                if remains_locked {
                    held_course = Some(previous_path);
                }
                remains_locked
            });
            let was_holding_course = held_course.is_some();
            let mut best = if let Some(course) = held_course {
                // Commitment has already been forecast at the full horizon above.
                // Reusing it avoids rescanning 360° while the route remains valid.
                trajectory_evaluations += carpet_evaluations;
                Some(course)
            } else {
                let scan_origin = if self.strategy == Strategy::AgileTop1 {
                    0.0
                } else {
                    forward_heading(transport, desert)
                };
                let mut coarse_angles = if self.strategy == Strategy::AgileTop1 {
                    full_circle_angles(0.0, AGILE_SCAN_ANGLE_STEP_DEG)
                } else {
                    primary_scan_angles(scan_origin)
                };
                if let Some(previous) = memo.angle {
                    push_unique(&mut coarse_angles, previous);
                }
                if available_target.is_none() {
                    if let Some(resume_angle) = memo.resume_angle {
                        push_unique(&mut coarse_angles, resume_angle);
                    }
                }
                let mut evaluations = score_angles(
                    desert,
                    transport,
                    &coarse_angles,
                    delay,
                    self.horizon,
                    &bounties,
                    &reserved_targets,
                    memo.angle,
                    held_target,
                    bounty_grid.as_ref(),
                    self.strategy == Strategy::StableProfit,
                    false,
                );
                carpet_evaluations += coarse_angles.len();

                let mut fine_angles = Vec::new();
                if self.strategy == Strategy::StableProfit {
                    // Rank candidates from the full-circle pass, then spend fine-grained
                    // simulation only around a few separated local basins.
                    let seeds = refinement_seeds(&evaluations);
                    for seed in seeds {
                        for angle in refinement_angles(seed) {
                            push_unique(&mut fine_angles, angle);
                        }
                    }
                    if let Some(previous) = memo.angle {
                        let covered = fine_angles.iter().any(|angle| {
                            angular_delta(*angle, previous).abs()
                                <= REFINE_ANGLE_STEP_DEG.to_radians() * 0.51
                        });
                        if !covered {
                            for angle in refinement_angles(previous) {
                                push_unique(&mut fine_angles, angle);
                            }
                        }
                    }
                    if available_target.is_none() {
                        if let Some(resume_angle) = memo.resume_angle {
                            let covered = fine_angles.iter().any(|angle| {
                                angular_delta(*angle, resume_angle).abs()
                                    <= REFINE_ANGLE_STEP_DEG.to_radians() * 0.51
                            });
                            if !covered {
                                for angle in refinement_angles(resume_angle) {
                                    push_unique(&mut fine_angles, angle);
                                }
                            }
                        }
                    }
                }
                carpet_evaluations += fine_angles.len();
                evaluations.extend(score_angles(
                    desert,
                    transport,
                    &fine_angles,
                    delay,
                    self.horizon,
                    &bounties,
                    &reserved_targets,
                    memo.angle,
                    held_target,
                    bounty_grid.as_ref(),
                    self.strategy == Strategy::StableProfit,
                    false,
                ));
                trajectory_evaluations += carpet_evaluations;
                let mut selected = evaluations
                    .iter()
                    .max_by(|a, b| {
                        if self.strategy == Strategy::AgileTop1 {
                            compare_agile_top1(a, b)
                        } else {
                            compare_success(a, b)
                        }
                    })
                    .cloned();
                if self.strategy == Strategy::StableProfit {
                    if let (Some(previous_angle), Some(current_best)) =
                        (memo.angle, selected.as_ref())
                    {
                        if let Some(incumbent) = evaluations
                            .iter()
                            .find(|candidate| {
                                angular_delta(candidate.angle, previous_angle).abs() < 1e-9
                            })
                            .filter(|path| path.death_at.is_none())
                        {
                            if current_best.death_at.is_none()
                                && !materially_more_profitable(
                                    incumbent.score_rate,
                                    current_best.score_rate,
                                )
                            {
                                selected = Some(incumbent.clone());
                            }
                        }
                    }
                }
                selected
            };
            let mut resume_intent = available_target
                .is_some()
                .then(|| memo.resume_angle.zip(memo.resume_target))
                .flatten();
            if let Some(held) = best.as_ref().filter(|_| was_holding_course) {
                // During a committed route, cheaply inspect a short tactical horizon for a
                // materially better immediate capture. The full horizon is still used to
                // validate every refined detour candidate, especially for survival.
                if resume_intent.is_none() {
                    let tactical = score_angles(
                        desert,
                        transport,
                        &full_circle_angles(held.angle, TACTICAL_ANGLE_STEP_DEG),
                        delay,
                        TACTICAL_HORIZON_SECONDS,
                        &bounties,
                        &reserved_targets,
                        Some(held.angle),
                        None,
                        None,
                        false,
                        false,
                    );
                    let seeds = refinement_seeds(&tactical);
                    let mut fine_angles = Vec::new();
                    for seed in seeds {
                        for angle in refinement_angles(seed) {
                            push_unique(&mut fine_angles, angle);
                        }
                    }
                    let tactical_fine = score_angles(
                        desert,
                        transport,
                        &fine_angles,
                        delay,
                        TACTICAL_HORIZON_SECONDS,
                        &bounties,
                        &reserved_targets,
                        Some(held.angle),
                        None,
                        None,
                        false,
                        false,
                    );
                    trajectory_evaluations += tactical.len() + tactical_fine.len();
                    let detour_angle = tactical_fine
                        .iter()
                        .filter(|candidate| {
                            candidate.death_at.is_none()
                                && candidate.bounty_count > 0
                                && candidate.target.as_ref().is_some_and(|target| {
                                    !held
                                        .target
                                        .as_ref()
                                        .is_some_and(|current| same_bounty(target, current))
                                })
                        })
                        .max_by(|a, b| a.first_bounty_rate.total_cmp(&b.first_bounty_rate))
                        .map(|candidate| candidate.angle);
                    if let Some(angle) = detour_angle {
                        let mut confirmed = score_angles(
                            desert,
                            transport,
                            &[angle],
                            delay,
                            self.horizon,
                            &bounties,
                            &reserved_targets,
                            Some(held.angle),
                            None,
                            None,
                            false,
                            false,
                        );
                        trajectory_evaluations += confirmed.len();
                        if let Some(detour) = confirmed
                            .drain(..)
                            .filter(|candidate| {
                                candidate.death_at.is_none()
                                    && candidate.bounty_count > 0
                                    && candidate.time_to_reach_score <= TACTICAL_HORIZON_SECONDS
                                    && candidate.first_bounty_rate
                                        >= held.first_bounty_rate
                                            + PROFIT_SWITCH_MIN_GAIN_PER_SECOND.max(
                                                held.first_bounty_rate
                                                    * PROFIT_SWITCH_RELATIVE_GAIN,
                                            )
                                    && candidate.target.as_ref().is_some_and(|target| {
                                        !held
                                            .target
                                            .as_ref()
                                            .is_some_and(|current| same_bounty(target, current))
                                    })
                            })
                            .max_by(|a, b| a.first_bounty_rate.total_cmp(&b.first_bounty_rate))
                        {
                            resume_intent = held.target.clone().map(|target| (held.angle, target));
                            best = Some(detour);
                        }
                    }
                }
            }
            let mut movement_decision = "aim";
            let mut selected_risk = best.as_ref().map(|candidate| candidate.risk_exposure);
            if self.movement_strategy == MovementStrategy::Survival {
                if let Some(aim_course) = best.as_ref() {
                    let baseline = evaluate(
                        desert,
                        transport,
                        aim_course.angle,
                        delay,
                        MOVEMENT_SAFETY_HORIZON_SECONDS,
                        &[],
                        &reserved_targets,
                        None,
                        None,
                        false,
                        true,
                    );
                    trajectory_evaluations += 1;
                    selected_risk = Some(baseline.risk_exposure);
                    if baseline.death_at.is_some() {
                        let safety_bounties = desert.bounties.clone();
                        let safety_grid =
                            BountyGrid::new(&safety_bounties, desert.transport_radius);
                        let safety_angles = full_circle_angles(0.0, MOVEMENT_SCAN_STEP_DEG);
                        let safety_routes = score_angles(
                            desert,
                            transport,
                            &safety_angles,
                            delay,
                            MOVEMENT_SAFETY_HORIZON_SECONDS,
                            &safety_bounties,
                            &reserved_targets,
                            None,
                            None,
                            Some(&safety_grid),
                            false,
                            true,
                        );
                        trajectory_evaluations += safety_routes.len();
                        let safe_routes: Vec<_> = safety_routes
                            .iter()
                            .filter(|candidate| candidate.death_at.is_none())
                            .collect();
                        let escape_trigger = survival_escape_trigger(
                            baseline.death_at,
                            safe_routes.len(),
                            safety_routes.len(),
                        );
                        if baseline.death_at.is_some() && escape_trigger.is_none() {
                            movement_decision = "aim-options-open";
                        }
                        if let Some(trigger) = escape_trigger {
                            let collecting_safe_routes: Vec<_> = safe_routes
                                .iter()
                                .copied()
                                .filter(|candidate| candidate.bounty_count > 0)
                                .collect();
                            let escape = collecting_safe_routes
                                .into_iter()
                                .max_by(|a, b| compare_agile_top1(a, b))
                                .or_else(|| {
                                    safe_routes
                                        .into_iter()
                                        .max_by(|a, b| compare_agile_top1(a, b))
                                })
                                .cloned();
                            if let Some(escape) = escape {
                                selected_risk = Some(escape.risk_exposure);
                                best = Some(escape);
                                movement_decision = match trigger {
                                    EscapeTrigger::Imminent => "escape-imminent",
                                    EscapeTrigger::ScarceOptions => "escape-scarce-options",
                                };
                            } else if let Some(doomed_route) = match self.doom_policy {
                                DoomPolicy::Collect => safety_routes
                                    .iter()
                                    .max_by(|a, b| compare_doom_collect(a, b)),
                                DoomPolicy::Fastest => {
                                    safety_routes.iter().max_by(|a, b| compare_success(a, b))
                                }
                            }
                            .cloned()
                            {
                                selected_risk = Some(doomed_route.risk_exposure);
                                best = Some(doomed_route);
                                movement_decision = match self.doom_policy {
                                    DoomPolicy::Collect => "no-escape-collect",
                                    DoomPolicy::Fastest => "no-escape-fastest",
                                };
                            }
                        }
                    }
                }
            }
            let Some(best) = best.take() else {
                continue;
            };
            let acceleration_magnitude = valid_command_accel(desert.max_accel);
            let acceleration = Vec2 {
                x: best.angle.cos(),
                y: best.angle.sin(),
            } * acceleration_magnitude;
            // Only a bounty actually collected by this route becomes a lock target;
            // near-miss potential remains a planning hint, not a commitment.
            let target = (best.bounty_count > 0)
                .then(|| best.target.clone())
                .flatten();
            let keep_stall = target
                .as_ref()
                .zip(memo.target.as_ref())
                .is_some_and(|(new_target, old_target)| same_bounty(new_target, old_target));
            if let Some(target) = &target {
                reserved_targets.push(target.position);
            }
            if self.strategy == Strategy::StableProfit {
                self.plans.insert(
                    transport.id.clone(),
                    PlanMemo {
                        angle: Some(best.angle),
                        target,
                        target_eta: (best.bounty_count > 0).then_some(best.time_to_reach_score),
                        stalled_ticks: if was_holding_course && keep_stall {
                            stalled_ticks
                        } else {
                            0
                        },
                        resume_angle: resume_intent.as_ref().map(|(angle, _)| *angle),
                        resume_target: resume_intent.map(|(_, target)| target),
                    },
                );
            }
            let death_reason = best.death_reason.map(|reason| match reason {
                DeathReason::Anomaly => "anomaly",
                DeathReason::Boundary => "boundary",
                DeathReason::Transport => "transport",
            });
            if let Some(row) = carpets.iter_mut().find(|row| row.id == transport.id) {
                row.command_acceleration = Some(acceleration);
                row.goal = Some(best.goal.label());
                row.target = best.target.as_ref().map(|target| target.position);
                row.target_points = best.target.as_ref().map(|target| target.points);
                row.target_distance = best
                    .target
                    .as_ref()
                    .map(|target| transport.position.distance(target.position));
                row.total_potential_score = Some(best.total_potential_score);
                row.route_score = Some(best.route_score);
                row.time_to_last_bounty = best.time_to_last_bounty;
                row.score_rate = Some(best.score_rate);
                row.time_to_reach_score = Some(best.time_to_reach_score);
                row.first_bounty_rate = Some(best.first_bounty_rate);
                row.bounty_count = best.bounty_count;
                row.death_at = best.death_at;
                row.death_reason = death_reason;
                row.risk_exposure = selected_risk;
                row.movement_decision = Some(movement_decision);
            }
            commands.push(Command {
                id: transport.id.clone(),
                acceleration,
            });
        }
        carpets.sort_by(|a, b| a.id.cmp(&b.id));
        PlanResult {
            commands,
            carpets,
            trajectory_evaluations,
        }
    }
}

fn score_angles(
    desert: &Desert,
    transport: &Transport,
    angles: &[f64],
    delay: f64,
    horizon: f64,
    bounties: &[Bounty],
    reserved_targets: &[Vec2],
    previous_angle: Option<f64>,
    held_target: Option<&Bounty>,
    bounty_grid: Option<&BountyGrid>,
    track_near_miss: bool,
    track_risk: bool,
) -> Vec<Evaluation> {
    angles
        .iter()
        .copied()
        .map(|angle| {
            let mut result = evaluate(
                desert,
                transport,
                angle,
                delay,
                horizon,
                bounties,
                reserved_targets,
                held_target,
                bounty_grid,
                track_near_miss,
                track_risk,
            );
            if let Some(previous) = previous_angle {
                let delta = angular_delta(result.angle, previous).abs();
                result.continuity = (1.0 - delta / PI).max(0.0);
                let change_cost = COURSE_CHANGE_MIN_COST
                    .max(result.total_potential_score.abs() * COURSE_CHANGE_SCORE_RATIO)
                    * (delta.to_degrees() / COURSE_CHANGE_FULL_COST_DEGREES).clamp(0.0, 1.0);
                result.selection_utility = result.total_potential_score
                    + TARGET_TIME_BONUS
                        * (-result.time_to_reach_score / TARGET_TIME_SCALE_SECONDS).exp()
                    - change_cost;
            } else {
                result.selection_utility = result.total_potential_score
                    + TARGET_TIME_BONUS
                        * (-result.time_to_reach_score / TARGET_TIME_SCALE_SECONDS).exp();
            }
            result
        })
        .collect()
}

fn evaluate(
    desert: &Desert,
    transport: &Transport,
    angle: f64,
    delay: f64,
    horizon: f64,
    bounties: &[Bounty],
    reserved_targets: &[Vec2],
    held_target: Option<&Bounty>,
    bounty_grid: Option<&BountyGrid>,
    track_near_miss: bool,
    track_risk: bool,
) -> Evaluation {
    let acceleration = Vec2 {
        x: angle.cos(),
        y: angle.sin(),
    } * valid_command_accel(desert.max_accel);
    let mut position = transport.position;
    let mut velocity = transport.velocity;
    let mut elapsed = 0.0;
    let mut speed_integral = 0.0;
    let mut speed_duration = 0.0;
    let mut risk_exposure = 0.0;
    let mut death_at = None;
    let mut death_reason = None;

    // Account for command transit and phase to the next server tick by holding the
    // last effective command until the new command can be applied.
    let mut lead_remaining = delay;
    while lead_remaining > 1e-9 {
        let dt = lead_remaining.min(TICK);
        let (next, next_velocity) = physics_step(
            position,
            velocity,
            transport.self_acceleration,
            dt,
            elapsed,
            desert,
        );
        let mut segment_death = death_on_segment(desert, transport, position, next, elapsed, dt);
        if track_risk && segment_death.is_none() {
            let risk = estimated_death_risk(
                (position + next) * 0.5,
                (velocity + next_velocity) * 0.5,
                elapsed + dt * 0.5,
                desert,
            );
            if risk >= 1.0 {
                segment_death = Some((0.5, DeathReason::Anomaly));
            }
        }
        if let Some((fraction, reason)) = segment_death {
            let duration = dt * fraction;
            let death_velocity = velocity + (next_velocity - velocity) * fraction;
            if track_risk {
                risk_exposure += estimated_death_risk(
                    (position + next) * 0.5,
                    (velocity + next_velocity) * 0.5,
                    elapsed + duration * 0.5,
                    desert,
                ) * duration;
            }
            speed_integral += (velocity.length() + death_velocity.length()) * 0.5 * duration;
            speed_duration += duration;
            velocity = death_velocity;
            death_at = Some(elapsed + dt * fraction);
            death_reason = Some(reason);
            position = position + (next - position) * fraction;
            elapsed += dt * fraction;
            break;
        }
        if track_risk {
            risk_exposure += estimated_death_risk(
                (position + next) * 0.5,
                (velocity + next_velocity) * 0.5,
                elapsed + dt * 0.5,
                desert,
            ) * dt;
        }
        speed_integral += (velocity.length() + next_velocity.length()) * 0.5 * dt;
        speed_duration += dt;
        position = next;
        velocity = next_velocity;
        elapsed += dt;
        lead_remaining -= dt;
    }

    let start_of_candidate = elapsed;
    let mut collected = vec![false; bounties.len()];
    let mut min_distances = if track_near_miss {
        vec![f64::INFINITY; bounties.len()]
    } else {
        Vec::new()
    };
    let mut closest_times = if track_near_miss {
        vec![f64::INFINITY; bounties.len()]
    } else {
        Vec::new()
    };
    let mut total_potential_score = 0.0;
    let mut route_score = 0.0;
    let mut time_to_last_bounty: Option<f64> = None;
    let mut bounty_count = 0;
    let mut target: Option<Bounty> = None;
    let mut target_time = f64::INFINITY;
    let mut held_target_collected = false;
    let mut held_target_time = None;
    let mut candidate_elapsed = 0.0;

    if death_at.is_none() {
        while candidate_elapsed + 1e-9 < horizon {
            let dt = (horizon - candidate_elapsed).min(TICK);
            let absolute_time = start_of_candidate + candidate_elapsed;
            let previous = position;
            let (new_position, new_velocity) =
                physics_step(position, velocity, acceleration, dt, absolute_time, desert);

            let mut collision =
                death_on_segment(desert, transport, previous, new_position, absolute_time, dt);
            if track_risk && collision.is_none() {
                let risk = estimated_death_risk(
                    (previous + new_position) * 0.5,
                    (velocity + new_velocity) * 0.5,
                    absolute_time + dt * 0.5,
                    desert,
                );
                if risk >= 1.0 {
                    collision = Some((0.5, DeathReason::Anomaly));
                }
            }
            let reward_end = collision
                .map(|(fraction, _)| previous + (new_position - previous) * fraction)
                .unwrap_or(new_position);
            let risk_duration = collision.map_or(dt, |(fraction, _)| dt * fraction);
            let risk_position = (previous + reward_end) * 0.5;
            let risk_velocity = (velocity + new_velocity) * 0.5;
            if track_risk {
                risk_exposure += estimated_death_risk(
                    risk_position,
                    risk_velocity,
                    absolute_time + risk_duration * 0.5,
                    desert,
                ) * risk_duration;
            }

            let mut inspect_bounty = |index: usize| {
                let bounty = &bounties[index];
                if collected[index] || is_reserved(bounty.position, reserved_targets) {
                    return;
                }
                let relative_start = bounty.position - previous;
                let relative_end = bounty.position - reward_end;
                if track_near_miss {
                    let (distance, fraction_to_closest) =
                        segment_min_distance(relative_start, relative_end);
                    if distance < min_distances[index] {
                        min_distances[index] = distance;
                        closest_times[index] = absolute_time + dt * fraction_to_closest;
                    }
                }
                // Match the server's swept check against the combined carpet/coin radius.
                let capture_radius = desert.transport_radius.max(0.0) + bounty.radius.max(0.0);
                let capture_entry =
                    segment_circle_entry(relative_start, relative_end, capture_radius);
                if collision.is_none() && capture_entry.is_some() {
                    collected[index] = true;
                    bounty_count += 1;
                    let event_time = absolute_time + dt * capture_entry.unwrap();
                    total_potential_score += bounty.points * (-event_time / 15.0).exp();
                    route_score += bounty.points;
                    time_to_last_bounty = Some(
                        time_to_last_bounty.map_or(event_time, |previous| previous.max(event_time)),
                    );
                    if event_time < target_time {
                        target = Some(bounty.clone());
                        target_time = event_time;
                    }
                    if held_target.is_some_and(|held| same_bounty(held, bounty)) {
                        held_target_collected = true;
                        held_target_time = Some(event_time);
                    }
                }
            };
            if let Some(grid) = bounty_grid {
                grid.visit_segment(previous, reward_end, &mut inspect_bounty);
            } else {
                for index in 0..bounties.len() {
                    inspect_bounty(index);
                }
            }

            if let Some((fraction, reason)) = collision {
                let duration = dt * fraction;
                let death_velocity = velocity + (new_velocity - velocity) * fraction;
                speed_integral += (velocity.length() + death_velocity.length()) * 0.5 * duration;
                speed_duration += duration;
                velocity = death_velocity;
                death_at = Some(candidate_elapsed + dt * fraction);
                death_reason = Some(reason);
                position = previous + (new_position - previous) * fraction;
                break;
            }

            speed_integral += (velocity.length() + new_velocity.length()) * 0.5 * dt;
            speed_duration += dt;
            velocity = new_velocity;
            position = new_position;
            candidate_elapsed += dt;
        }
    }

    let mut potential = 0.0;
    let mut potential_target = None;
    let mut potential_target_time = f64::INFINITY;
    if track_near_miss {
        for (index, bounty) in bounties.iter().enumerate() {
            if collected[index] || is_reserved(bounty.position, reserved_targets) {
                continue;
            }
            let capture_radius = desert.transport_radius.max(0.0) + bounty.radius.max(0.0);
            let miss_distance = (min_distances[index] - capture_radius).max(0.0);
            let near_miss_ratio = (1.0 - miss_distance / BOUNTY_AIM_BUFFER).clamp(0.0, 1.0);
            let value = bounty.points * (-min_distances[index] / 450.0).exp() * 0.3
                + bounty.points * 0.05 * near_miss_ratio;
            if value > potential {
                potential = value;
                potential_target = Some(bounty.clone());
                potential_target_time =
                    closest_times[index] + min_distances[index] / desert.max_speed.max(1.0);
            }
        }
    }
    let has_potential_target = potential_target.is_some();
    if held_target_collected {
        target = held_target.cloned();
        target_time = held_target_time.unwrap_or(target_time);
    } else if target.is_none() {
        target = potential_target;
    }
    if bounties.is_empty() {
        potential = -position.distance(Vec2 {
            x: desert.map_size.x * 0.5,
            y: desert.map_size.y * 0.5,
        }) / 100.0;
    }

    total_potential_score += potential;
    let time_to_reach_score = if target_time.is_finite() {
        target_time
    } else if has_potential_target {
        potential_target_time
    } else {
        start_of_candidate + horizon
    };
    let speed_limit = desert.max_speed.max(1.0);
    let average_speed_ratio = if speed_duration > 0.0 {
        (speed_integral / speed_duration / speed_limit).clamp(0.0, 1.0)
    } else {
        (velocity.length() / speed_limit).clamp(0.0, 1.0)
    };
    let terminal_speed_ratio = (velocity.length() / speed_limit).clamp(0.0, 1.0);

    Evaluation {
        angle,
        goal: if death_at.is_some() {
            Goal::FastestDeath
        } else if bounty_count > 0 {
            Goal::CollectBounties
        } else {
            Goal::AvoidDeath
        },
        total_potential_score,
        route_score,
        time_to_last_bounty,
        score_rate: score_rate(route_score, time_to_last_bounty),
        first_bounty_rate: target
            .as_ref()
            .filter(|_| target_time.is_finite())
            .map(|bounty| bounty.points.max(0.0) / target_time.max(TICK))
            .unwrap_or(0.0),
        time_to_reach_score,
        selection_utility: total_potential_score
            + TARGET_TIME_BONUS * (-time_to_reach_score / TARGET_TIME_SCALE_SECONDS).exp(),
        average_speed_ratio,
        terminal_speed_ratio,
        bounty_count,
        continuity: 0.0,
        keeps_target: held_target_collected,
        target,
        death_at,
        death_reason,
        risk_exposure,
    }
}

/// Compare trajectory success lexicographically so reward can never outweigh survival.
fn compare_success(a: &Evaluation, b: &Evaluation) -> Ordering {
    match (a.death_at, b.death_at) {
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a_time), Some(b_time)) => a
            .average_speed_ratio
            .total_cmp(&b.average_speed_ratio)
            .then_with(|| a.terminal_speed_ratio.total_cmp(&b.terminal_speed_ratio))
            .then_with(|| b_time.total_cmp(&a_time))
            .then_with(|| a.total_potential_score.total_cmp(&b.total_potential_score))
            .then_with(|| a.bounty_count.cmp(&b.bounty_count))
            .then_with(|| b.continuity.total_cmp(&a.continuity)),
        (None, None) => compare_safe_success(a, b),
    }
}

/// Rank agile candidates the same way the visualizer ranks its profitable route fan.
/// Safety remains a hard constraint before score-rate, so a safe empty route beats death.
fn compare_agile_top1(a: &Evaluation, b: &Evaluation) -> Ordering {
    match (a.death_at, b.death_at) {
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(_), Some(_)) => compare_success(a, b),
        (None, None) => a
            .score_rate
            .total_cmp(&b.score_rate)
            .then_with(|| a.route_score.total_cmp(&b.route_score))
            .then_with(|| a.average_speed_ratio.total_cmp(&b.average_speed_ratio))
            .then_with(|| a.terminal_speed_ratio.total_cmp(&b.terminal_speed_ratio))
            .then_with(|| b.time_to_reach_score.total_cmp(&a.time_to_reach_score)),
    }
}

/// In a no-escape horizon, collect policy buys time for actual bounty captures first.
fn compare_doom_collect(a: &Evaluation, b: &Evaluation) -> Ordering {
    a.route_score
        .total_cmp(&b.route_score)
        .then_with(|| a.score_rate.total_cmp(&b.score_rate))
        .then_with(|| {
            a.death_at
                .unwrap_or(0.0)
                .total_cmp(&b.death_at.unwrap_or(0.0))
        })
        .then_with(|| compare_success(a, b))
}

fn compare_safe_success(a: &Evaluation, b: &Evaluation) -> Ordering {
    match a.keeps_target.cmp(&b.keeps_target) {
        Ordering::Equal => {}
        order => return order,
    }

    let speed_delta = a.average_speed_ratio - b.average_speed_ratio;
    if speed_delta.abs() > SPEED_TIE_EPSILON {
        return speed_delta.total_cmp(&0.0);
    }

    a.selection_utility
        .total_cmp(&b.selection_utility)
        .then_with(|| a.score_rate.total_cmp(&b.score_rate))
        .then_with(|| a.continuity.total_cmp(&b.continuity))
        .then_with(|| a.average_speed_ratio.total_cmp(&b.average_speed_ratio))
        .then_with(|| a.terminal_speed_ratio.total_cmp(&b.terminal_speed_ratio))
        .then_with(|| a.total_potential_score.total_cmp(&b.total_potential_score))
        .then_with(|| b.time_to_reach_score.total_cmp(&a.time_to_reach_score))
        .then_with(|| a.bounty_count.cmp(&b.bounty_count))
}

fn materially_more_profitable(incumbent_rate: f64, candidate_rate: f64) -> bool {
    if candidate_rate <= incumbent_rate {
        return false;
    }
    let required_gain =
        PROFIT_SWITCH_MIN_GAIN_PER_SECOND.max(incumbent_rate * PROFIT_SWITCH_RELATIVE_GAIN);
    candidate_rate - incumbent_rate >= required_gain
}

fn score_rate(route_score: f64, time_to_last_bounty: Option<f64>) -> f64 {
    let Some(duration) = time_to_last_bounty else {
        return 0.0;
    };
    route_score.max(0.0) / duration.max(TICK)
}

/// Keep every alive carpet's command non-zero, even if a malformed snapshot
/// advertises a zero acceleration limit. Valid worlds use their exact maxAccel.
fn valid_command_accel(max_accel: f64) -> f64 {
    if max_accel.is_finite() && max_accel > 0.0 {
        max_accel
    } else {
        1.0e-6
    }
}

fn physics_step(
    position: Vec2,
    velocity: Vec2,
    acceleration: Vec2,
    dt: f64,
    time: f64,
    desert: &Desert,
) -> (Vec2, Vec2) {
    let env = environmental_force(position, time, &desert.anomalies, desert.map_size);
    let friction = FRICTION.powf(dt / TICK);
    let next_velocity =
        (velocity * friction + (acceleration + env) * dt).clamp(desert.max_speed.max(0.0));
    (position + next_velocity * dt, next_velocity)
}

fn environmental_force(position: Vec2, time: f64, anomalies: &[Anomaly], map: Vec2) -> Vec2 {
    let mut total = Vec2::default();
    for anomaly in anomalies {
        let center = anomaly.position + anomaly.velocity * time;
        if !anomaly_overlaps_arena(center, anomaly.effect_radius, map) {
            continue;
        }
        let delta = center - position;
        let distance = delta.length();
        if distance > 1e-9 && distance <= anomaly.effect_radius {
            total = total + delta * (anomaly.strength / distance);
        }
    }
    total
}

/// Returns normalized local danger density in [0, 1]. It rises near lethal
/// attraction cores and where anomaly force/velocity drives the carpet toward an edge.
fn estimated_death_risk(position: Vec2, velocity: Vec2, time: f64, desert: &Desert) -> f64 {
    // The movement policy's danger field is evaluated on a 350-world-unit grid.
    // Exact collision/death checks still use continuous positions at every physics step.
    let position = Vec2 {
        x: ((position.x / MOVEMENT_RISK_GRID_STEP).floor() + 0.5) * MOVEMENT_RISK_GRID_STEP,
        y: ((position.y / MOVEMENT_RISK_GRID_STEP).floor() + 0.5) * MOVEMENT_RISK_GRID_STEP,
    };
    if guaranteed_death_cell(position, time, desert, 4) {
        return 1.0;
    }
    let max_accel = valid_command_accel(desert.max_accel);
    let max_speed = desert.max_speed.max(1.0);
    let force = environmental_force(position, time, &desert.anomalies, desert.map_size);
    let mut risk: f64 = 0.0;

    for anomaly in desert
        .anomalies
        .iter()
        .filter(|anomaly| anomaly.strength > 0.0)
    {
        let center = anomaly.position + anomaly.velocity * time;
        let delta = center - position;
        let distance = delta.length();
        let lethal_radius = anomaly.core_radius + desert.transport_radius;
        if distance <= lethal_radius {
            return 1.0;
        }
        let escape_span = (anomaly.effect_radius - lethal_radius).max(1.0);
        if distance > anomaly.effect_radius || distance <= 1e-9 {
            continue;
        }
        let toward_core = delta * (1.0 / distance);
        let inward_force = (force.x * toward_core.x + force.y * toward_core.y).max(0.0);
        let pressure = inward_force / max_accel;
        let proximity = ((anomaly.effect_radius - distance) / escape_span).clamp(0.0, 1.0);
        let trap_risk = if pressure > 1.0 {
            0.75 + 0.25 * proximity
        } else {
            (0.75 * pressure * proximity).clamp(0.0, 0.75)
        };
        risk = risk.max(trap_risk);
    }

    let edge_horizon = (max_speed * 2.0).max(1.0);
    for (clearance, outward_speed, outward_force) in [
        (position.x, -velocity.x, -force.x),
        (desert.map_size.x - position.x, velocity.x, force.x),
        (position.y, -velocity.y, -force.y),
        (desert.map_size.y - position.y, velocity.y, force.y),
    ] {
        let closeness = (-clearance.max(0.0) / edge_horizon).exp();
        let speed_pressure = outward_speed.max(0.0) / max_speed;
        let force_pressure = outward_force.max(0.0) / max_accel;
        let outward_pressure = speed_pressure + force_pressure;
        let edge_risk = if force_pressure > 1.0 {
            closeness * (0.75 + 0.25 * (1.0 - (-force_pressure).exp()))
        } else {
            (closeness * outward_pressure * 0.5).clamp(0.0, 0.75)
        };
        risk = risk.max(edge_risk);
    }
    risk.clamp(0.0, 0.99)
}

fn guaranteed_death_cell(position: Vec2, time: f64, desert: &Desert, chain_depth: u8) -> bool {
    if position.x < 0.0
        || position.y < 0.0
        || position.x > desert.map_size.x
        || position.y > desert.map_size.y
    {
        return true;
    }
    let control = valid_command_accel(desert.max_accel);
    for anomaly in &desert.anomalies {
        let center = anomaly.position + anomaly.velocity * time;
        let delta = center - position;
        let distance = delta.length();
        let lethal_radius = anomaly.core_radius + desert.transport_radius;
        if distance <= lethal_radius {
            return true;
        }
        if anomaly.strength <= 0.0 || distance > anomaly.effect_radius || distance <= 1e-9 {
            continue;
        }
        let proximity = ((anomaly.effect_radius - distance)
            / (anomaly.effect_radius - lethal_radius).max(1.0))
        .clamp(0.0, 1.0);
        let force = environmental_force(position, time, &desert.anomalies, desert.map_size);
        let inward_force = (force.x * delta.x + force.y * delta.y) / distance;
        if inward_force > control && proximity >= 0.8 {
            return true;
        }
    }

    let force = environmental_force(position, time, &desert.anomalies, desert.map_size);
    let edge_horizon = desert.max_speed.max(1.0) * 2.0;
    for (clearance, outward_force) in [
        (position.x, -force.x),
        (desert.map_size.x - position.x, force.x),
        (position.y, -force.y),
        (desert.map_size.y - position.y, force.y),
    ] {
        if outward_force > control && clearance <= edge_horizon {
            return true;
        }
    }

    if chain_depth > 0 && force.length() > control {
        let next = position + force * (MOVEMENT_RISK_GRID_STEP / force.length());
        if guaranteed_death_cell(next, time, desert, chain_depth - 1) {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EscapeTrigger {
    Imminent,
    ScarceOptions,
}

fn survival_escape_trigger(
    death_at: Option<f64>,
    safe_route_count: usize,
    total_route_count: usize,
) -> Option<EscapeTrigger> {
    let death_at = death_at?;
    if death_at <= MOVEMENT_ESCAPE_IMMINENT_SECONDS {
        return Some(EscapeTrigger::Imminent);
    }
    if total_route_count > 0
        && safe_route_count.saturating_mul(100)
            <= total_route_count.saturating_mul(MOVEMENT_SCARCE_SAFE_ROUTE_PERCENT)
    {
        return Some(EscapeTrigger::ScarceOptions);
    }
    None
}

fn death_on_segment(
    desert: &Desert,
    transport: &Transport,
    start: Vec2,
    end: Vec2,
    time: f64,
    dt: f64,
) -> Option<(f64, DeathReason)> {
    let mut earliest: Option<(f64, DeathReason)> = boundary_entry(start, end, desert.map_size);
    for anomaly in &desert.anomalies {
        let center_start = anomaly.position + anomaly.velocity * time;
        let center_end = anomaly.position + anomaly.velocity * (time + dt);
        let center_mid = anomaly.position + anomaly.velocity * (time + dt * 0.5);
        if !anomaly_overlaps_arena(center_start, anomaly.effect_radius, desert.map_size)
            && !anomaly_overlaps_arena(center_mid, anomaly.effect_radius, desert.map_size)
            && !anomaly_overlaps_arena(center_end, anomaly.effect_radius, desert.map_size)
        {
            continue;
        }
        if let Some(fraction) = segment_circle_entry(
            center_start - start,
            center_end - end,
            anomaly.core_radius + desert.transport_radius,
        ) {
            retain_earliest(&mut earliest, fraction, DeathReason::Anomaly);
        }
    }
    let collision_radius = 2.0 * desert.transport_radius;
    for other in desert
        .transports
        .iter()
        .filter(|other| other.alive && other.id != transport.id)
    {
        let other_start = other.position + other.velocity * time;
        let other_end = other.position + other.velocity * (time + dt);
        if let Some(fraction) =
            segment_circle_entry(other_start - start, other_end - end, collision_radius)
        {
            retain_earliest(&mut earliest, fraction, DeathReason::Transport);
        }
    }
    for enemy in desert.enemies.iter().filter(|enemy| enemy.alive) {
        let enemy_start = enemy.position + enemy.velocity * time;
        let enemy_end = enemy.position + enemy.velocity * (time + dt);
        if let Some(fraction) =
            segment_circle_entry(enemy_start - start, enemy_end - end, collision_radius)
        {
            retain_earliest(&mut earliest, fraction, DeathReason::Transport);
        }
    }
    earliest
}

fn anomaly_overlaps_arena(center: Vec2, radius: f64, map: Vec2) -> bool {
    let closest = Vec2 {
        x: center.x.clamp(0.0, map.x),
        y: center.y.clamp(0.0, map.y),
    };
    center.distance(closest) <= radius
}

fn retain_earliest(slot: &mut Option<(f64, DeathReason)>, fraction: f64, reason: DeathReason) {
    if (0.0..=1.0).contains(&fraction) && slot.is_none_or(|(time, _)| fraction < time) {
        *slot = Some((fraction, reason));
    }
}

fn boundary_entry(start: Vec2, end: Vec2, map: Vec2) -> Option<(f64, DeathReason)> {
    if start.x < 0.0 || start.x > map.x || start.y < 0.0 || start.y > map.y {
        return Some((0.0, DeathReason::Boundary));
    }
    let delta = end - start;
    let mut candidates = Vec::with_capacity(2);
    if end.x < 0.0 && delta.x < 0.0 {
        candidates.push((0.0 - start.x) / delta.x);
    }
    if end.x > map.x && delta.x > 0.0 {
        candidates.push((map.x - start.x) / delta.x);
    }
    if end.y < 0.0 && delta.y < 0.0 {
        candidates.push((0.0 - start.y) / delta.y);
    }
    if end.y > map.y && delta.y > 0.0 {
        candidates.push((map.y - start.y) / delta.y);
    }
    candidates
        .into_iter()
        .filter(|t| (0.0..=1.0).contains(t))
        .min_by(f64::total_cmp)
        .map(|t| (t, DeathReason::Boundary))
}

fn segment_circle_entry(relative_start: Vec2, relative_end: Vec2, radius: f64) -> Option<f64> {
    if radius <= 0.0 {
        return None;
    }
    let movement = relative_end - relative_start;
    let c =
        relative_start.x * relative_start.x + relative_start.y * relative_start.y - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let a = movement.x * movement.x + movement.y * movement.y;
    if a <= 1e-12 {
        return None;
    }
    let b = 2.0 * (relative_start.x * movement.x + relative_start.y * movement.y);
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let entry = (-b - discriminant.sqrt()) / (2.0 * a);
    (0.0..=1.0).contains(&entry).then_some(entry)
}

fn segment_min_distance(relative_start: Vec2, relative_end: Vec2) -> (f64, f64) {
    let delta = relative_end - relative_start;
    let length_sq = delta.x * delta.x + delta.y * delta.y;
    let fraction = if length_sq <= 1e-12 {
        0.0
    } else {
        (-(relative_start.x * delta.x + relative_start.y * delta.y) / length_sq).clamp(0.0, 1.0)
    };
    ((relative_start + delta * fraction).length(), fraction)
}

fn shortlist_bounties(position: Vec2, bounties: &[Bounty], reserved: &[Vec2]) -> Vec<Bounty> {
    let mut nearest = bounties
        .iter()
        .filter(|b| !is_reserved(b.position, reserved))
        .cloned()
        .collect::<Vec<_>>();
    nearest.sort_by(|a, b| {
        position
            .distance(a.position)
            .total_cmp(&position.distance(b.position))
    });
    let mut selected = nearest
        .iter()
        .take(BOUNTY_NEAREST_SHORTLIST)
        .cloned()
        .collect::<Vec<_>>();
    let mut valuable = nearest;
    valuable.sort_by(|a, b| b.points.total_cmp(&a.points));
    for bounty in valuable.into_iter().take(BOUNTY_VALUABLE_SHORTLIST) {
        if selected.len() >= BOUNTY_SHORTLIST {
            break;
        }
        if !selected
            .iter()
            .any(|b| b.position.distance(bounty.position) < 1.0)
        {
            selected.push(bounty);
        }
    }
    selected
}

fn is_reserved(position: Vec2, reserved: &[Vec2]) -> bool {
    reserved.iter().any(|point| point.distance(position) < 1.0)
}

fn contains_bounty(bounties: &[Bounty], target: &Bounty) -> bool {
    bounties
        .iter()
        .any(|b| b.position.distance(target.position) < 1.0)
}

fn same_bounty(a: &Bounty, b: &Bounty) -> bool {
    a.position.distance(b.position) < 1.0
}

fn forward_heading(transport: &Transport, desert: &Desert) -> f64 {
    if transport.velocity.length() > 1.0 {
        return transport.velocity.y.atan2(transport.velocity.x);
    }
    desert
        .bounties
        .iter()
        .max_by(|a, b| {
            (a.points / (1.0 + transport.position.distance(a.position) / 500.0))
                .total_cmp(&(b.points / (1.0 + transport.position.distance(b.position) / 500.0)))
        })
        .map(|b| {
            (b.position - transport.position)
                .y
                .atan2((b.position - transport.position).x)
        })
        .unwrap_or_else(|| {
            let center = Vec2 {
                x: desert.map_size.x * 0.5,
                y: desert.map_size.y * 0.5,
            } - transport.position;
            center.y.atan2(center.x)
        })
}

fn full_circle_angles(center: f64, step_degrees: f64) -> Vec<f64> {
    let count = (360.0 / step_degrees).round() as usize;
    (0..count)
        .map(|index| (center + degrees_to_radians(index as f64 * step_degrees)).rem_euclid(TAU))
        .collect()
}

fn primary_scan_angles(center: f64) -> Vec<f64> {
    full_circle_angles(center, SCAN_ANGLE_STEP_DEG)
}

fn refinement_angles(center: f64) -> Vec<f64> {
    let mut result = Vec::new();
    let steps = (REFINE_HALF_WIDTH_DEG / REFINE_ANGLE_STEP_DEG).round() as i32;
    for step in -steps..=steps {
        let degrees = step as f64 * REFINE_ANGLE_STEP_DEG;
        push_unique(&mut result, center + degrees_to_radians(degrees));
    }
    result
}

fn refinement_seeds(evaluations: &[Evaluation]) -> Vec<f64> {
    let min_separation = (REFINE_HALF_WIDTH_DEG * 2.0).to_radians();
    let mut seeds = Vec::with_capacity(MAX_REFINEMENT_SEEDS);
    while seeds.len() < MAX_REFINEMENT_SEEDS {
        let next = evaluations
            .iter()
            .filter(|candidate| {
                seeds
                    .iter()
                    .all(|seed: &f64| angular_delta(*seed, candidate.angle).abs() >= min_separation)
            })
            .max_by(|a, b| compare_success(a, b));
        let Some(candidate) = next else { break };
        seeds.push(candidate.angle);
    }
    seeds
}

fn push_unique(angles: &mut Vec<f64>, angle: f64) {
    let normalized = angle.rem_euclid(TAU);
    if !angles
        .iter()
        .any(|existing| angular_delta(*existing, normalized).abs() < 1e-6)
    {
        angles.push(normalized);
    }
}

fn angular_delta(a: f64, b: f64) -> f64 {
    (a - b + PI).rem_euclid(TAU) - PI
}

fn degrees_to_radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

#[cfg(test)]
mod tests {
    use super::{
        angular_delta, boundary_entry, compare_agile_top1, compare_success, environmental_force,
        estimated_death_risk, evaluate, full_circle_angles, materially_more_profitable,
        physics_step, primary_scan_angles, refinement_angles, refinement_seeds, same_bounty,
        score_rate, segment_circle_entry, survival_escape_trigger, valid_command_accel, Bot,
        DeathReason, DoomPolicy, EscapeTrigger, Evaluation, MovementStrategy, Strategy, FRICTION,
        TICK,
    };
    use crate::model::{Anomaly, Bounty, Desert, Transport, Vec2};
    use std::cmp::Ordering;

    fn empty_desert() -> Desert {
        Desert {
            errors: vec![],
            map_size: Vec2 {
                x: 1000.0,
                y: 1000.0,
            },
            max_accel: 40.0,
            max_speed: 110.0,
            transport_radius: 5.0,
            transports: vec![],
            anomalies: vec![],
            bounties: vec![],
            enemies: vec![],
        }
    }

    #[test]
    fn physics_step_matches_server_euler_order() {
        let desert = empty_desert();
        let (position, velocity) = physics_step(
            Vec2 { x: 1.0, y: 2.0 },
            Vec2 { x: 10.0, y: -2.0 },
            Vec2 { x: 40.0, y: 0.0 },
            TICK,
            0.0,
            &desert,
        );
        assert!((velocity.x - (10.0 * FRICTION + 8.0)).abs() < 1e-9);
        assert!((velocity.y - (-2.0 * FRICTION)).abs() < 1e-9);
        assert!((position.x - (1.0 + velocity.x * TICK)).abs() < 1e-9);
        assert!((position.y - (2.0 + velocity.y * TICK)).abs() < 1e-9);
    }

    #[test]
    fn anomaly_strength_sign_controls_attraction_and_repulsion() {
        let anomaly = Anomaly {
            position: Vec2 { x: 10.0, y: 0.0 },
            velocity: Vec2::default(),
            core_radius: 2.0,
            effect_radius: 50.0,
            strength: 4.0,
        };
        let attracting = environmental_force(
            Vec2::default(),
            0.0,
            std::slice::from_ref(&anomaly),
            Vec2 { x: 100.0, y: 100.0 },
        );
        assert!((attracting.x - 4.0).abs() < 1e-9);
        let repelling = environmental_force(
            Vec2::default(),
            0.0,
            &[Anomaly {
                strength: -4.0,
                ..anomaly
            }],
            Vec2 { x: 100.0, y: 100.0 },
        );
        assert!((repelling.x + 4.0).abs() < 1e-9);
    }

    #[test]
    fn detects_circle_crossing_between_endpoints() {
        let entry =
            segment_circle_entry(Vec2 { x: -20.0, y: 0.0 }, Vec2 { x: 20.0, y: 0.0 }, 5.0).unwrap();
        assert!((entry - 0.375).abs() < 1e-9);
    }

    #[test]
    fn bounty_capture_matches_server_swept_combined_radii() {
        let transport = Transport {
            id: "carpet".into(),
            position: Vec2 { x: 100.0, y: 500.0 },
            velocity: Vec2 { x: 110.0, y: 0.0 },
            self_acceleration: Vec2::default(),
            anomaly_acceleration: Vec2::default(),
            alive: true,
        };
        let bounty = Bounty {
            // The endpoint is x=122, 11 units from center, while the segment
            // passes through the combined-radius (10) circle.
            position: Vec2 { x: 111.0, y: 500.0 },
            points: 100.0,
            radius: 5.0,
        };
        let desert = Desert {
            transports: vec![transport.clone()],
            bounties: vec![bounty],
            ..empty_desert()
        };
        let swept_capture = evaluate(
            &desert,
            &transport,
            0.0,
            0.0,
            TICK,
            &desert.bounties,
            &[],
            None,
            None,
            true,
            false,
        );
        assert_eq!(swept_capture.bounty_count, 1);
        assert_eq!(swept_capture.route_score, 100.0);

        let captured_bounty = Bounty {
            position: Vec2 {
                x: 111.0,
                y: 510.001,
            },
            radius: 5.0,
            ..desert.bounties[0].clone()
        };
        let capture_desert = Desert {
            bounties: vec![captured_bounty],
            ..desert.clone()
        };
        let captured = evaluate(
            &capture_desert,
            &transport,
            0.0,
            0.0,
            TICK,
            &capture_desert.bounties,
            &[],
            None,
            None,
            true,
            false,
        );
        assert_eq!(captured.bounty_count, 0);
        assert_eq!(captured.route_score, 0.0);
    }

    #[test]
    fn detects_map_boundary_crossing() {
        let (time, _) = boundary_entry(
            Vec2 { x: 9.0, y: 5.0 },
            Vec2 { x: 11.0, y: 5.0 },
            Vec2 { x: 10.0, y: 10.0 },
        )
        .unwrap();
        assert!((time - 0.5).abs() < 1e-9);
    }

    #[test]
    fn chosen_acceleration_has_maximum_magnitude() {
        let mut bot = Bot::new(2.0);
        let desert = Desert {
            transports: vec![Transport {
                id: "p_0".into(),
                position: Vec2 { x: 500.0, y: 500.0 },
                velocity: Vec2 { x: 5.0, y: 0.0 },
                self_acceleration: Vec2::default(),
                anomaly_acceleration: Vec2::default(),
                alive: true,
            }],
            anomalies: vec![],
            bounties: vec![Bounty {
                position: Vec2 { x: 700.0, y: 500.0 },
                points: 500.0,
                radius: 5.0,
            }],
            ..empty_desert()
        };
        let plan = bot.plan(&desert);
        assert!(plan.trajectory_evaluations >= 72);
        let command = plan.commands.into_iter().next().unwrap();
        assert!((command.acceleration.length() - 40.0).abs() < 1e-9);
    }

    #[test]
    fn zero_limit_still_produces_nonzero_acceleration_command() {
        let mut desert = empty_desert();
        desert.max_accel = 0.0;
        desert.transports.push(Transport {
            id: "alive".into(),
            position: Vec2 { x: 500.0, y: 500.0 },
            velocity: Vec2::default(),
            self_acceleration: Vec2::default(),
            anomaly_acceleration: Vec2::default(),
            alive: true,
        });
        let mut bot = Bot::new(1.0);
        let commands = bot.plan(&desert).commands;
        assert_eq!(commands.len(), 1);
        assert!(commands[0].acceleration.length() > 0.0);
        assert!(valid_command_accel(40.0) > 0.0);
    }

    #[test]
    fn each_scan_covers_360_degrees_at_the_documented_resolution() {
        let origin: f64 = 1.234;
        let coarse = full_circle_angles(origin, 5.0);
        let scan = full_circle_angles(origin, 0.25);
        let refinement = refinement_angles(origin);
        assert_eq!(coarse.len(), 72);
        assert_eq!(scan.len(), 1440);
        assert_eq!(refinement.len(), 25);
        assert!((coarse[0] - origin).abs() < 1e-9);
        let expected = (origin + 0.25_f64.to_radians()).rem_euclid(std::f64::consts::TAU);
        assert!((scan[1] - expected).abs() < 1e-9);
    }

    #[test]
    fn primary_scan_is_uniformly_dense_for_all_360_degrees() {
        let center = 0.7;
        let scan = primary_scan_angles(center);
        assert_eq!(scan.len(), 72);
        for pair in scan.windows(2) {
            assert!((angular_delta(pair[1], pair[0]).to_degrees() - 5.0).abs() < 1e-9);
        }
        assert!((angular_delta(scan[0], scan[71]).to_degrees() - 5.0).abs() < 1e-9);
        // The exact opposite of the forward axis is explicitly one scan candidate.
        assert!(scan
            .iter()
            .any(|angle| angular_delta(*angle, center).abs().to_degrees() > 179.0));
    }

    #[test]
    fn refinement_seeds_choose_three_separated_best_regions() {
        let candidate = |angle, score| Evaluation {
            angle,
            goal: super::Goal::AvoidDeath,
            total_potential_score: score,
            route_score: 0.0,
            time_to_last_bounty: None,
            score_rate: 0.0,
            first_bounty_rate: 0.0,
            time_to_reach_score: 0.0,
            selection_utility: score,
            average_speed_ratio: 1.0,
            terminal_speed_ratio: 1.0,
            bounty_count: 0,
            continuity: 0.0,
            keeps_target: false,
            target: None,
            death_at: None,
            death_reason: None,
            risk_exposure: 0.0,
        };
        let evaluations = vec![
            candidate(0.0, 10.0),
            candidate(2_f64.to_radians(), 9.0),
            candidate(15_f64.to_radians(), 8.0),
            candidate(30_f64.to_radians(), 7.0),
        ];
        let seeds = refinement_seeds(&evaluations);
        assert_eq!(seeds.len(), 3);
        assert!((seeds[0] - 0.0).abs() < 1e-9);
        assert!((seeds[1] - 15_f64.to_radians()).abs() < 1e-9);
        assert!((seeds[2] - 30_f64.to_radians()).abs() < 1e-9);
    }

    #[test]
    fn trajectory_success_prioritizes_survival_then_reward_and_fast_death() {
        let make = |death_at: Option<f64>, score: f64, speed: f64| Evaluation {
            angle: 0.0,
            goal: if death_at.is_some() {
                super::Goal::FastestDeath
            } else {
                super::Goal::AvoidDeath
            },
            total_potential_score: score,
            route_score: 0.0,
            time_to_last_bounty: None,
            score_rate: 1.0,
            first_bounty_rate: 0.0,
            time_to_reach_score: 1.0,
            selection_utility: score,
            average_speed_ratio: speed,
            terminal_speed_ratio: speed,
            bounty_count: 1,
            continuity: 0.0,
            keeps_target: false,
            target: None,
            death_at,
            death_reason: None,
            risk_exposure: 0.0,
        };
        let safe = make(None, 0.0, 1.0);
        let tempting_but_dead = make(Some(1.0), 1_000_000.0, 1.0);
        assert_eq!(
            compare_success(&safe, &tempting_but_dead),
            Ordering::Greater
        );
        let quick_death = make(Some(1.0), 0.0, 1.0);
        let slow_death_with_coin = make(Some(2.0), 1_000.0, 1.0);
        assert_eq!(
            compare_success(&quick_death, &slow_death_with_coin),
            Ordering::Greater
        );
    }

    #[test]
    fn safe_trajectories_prefer_speed_over_extra_bounty_score() {
        let mut fast = Evaluation {
            angle: 0.0,
            goal: super::Goal::AvoidDeath,
            total_potential_score: 0.0,
            route_score: 0.0,
            time_to_last_bounty: None,
            score_rate: 1.0,
            first_bounty_rate: 0.0,
            time_to_reach_score: 1.0,
            selection_utility: 0.0,
            average_speed_ratio: 1.0,
            terminal_speed_ratio: 1.0,
            bounty_count: 0,
            continuity: 0.0,
            keeps_target: false,
            target: None,
            death_at: None,
            death_reason: None,
            risk_exposure: 0.0,
        };
        let mut slower_for_coin = fast.clone();
        slower_for_coin.total_potential_score = 10_000.0;
        slower_for_coin.average_speed_ratio = 0.8;
        slower_for_coin.terminal_speed_ratio = 0.8;
        fast.total_potential_score = 0.0;
        assert_eq!(compare_success(&fast, &slower_for_coin), Ordering::Greater);
    }

    #[test]
    fn fatal_trajectories_also_prefer_maximum_speed_before_shortest_death() {
        let mut fast = Evaluation {
            angle: 0.0,
            goal: super::Goal::FastestDeath,
            total_potential_score: 0.0,
            route_score: 0.0,
            time_to_last_bounty: None,
            score_rate: 1.0,
            first_bounty_rate: 0.0,
            time_to_reach_score: 1.0,
            selection_utility: 0.0,
            average_speed_ratio: 1.0,
            terminal_speed_ratio: 1.0,
            bounty_count: 0,
            continuity: 0.0,
            keeps_target: false,
            target: None,
            death_at: Some(2.0),
            death_reason: None,
            risk_exposure: 0.0,
        };
        let mut slower_and_sooner = fast.clone();
        slower_and_sooner.average_speed_ratio = 0.7;
        slower_and_sooner.terminal_speed_ratio = 0.7;
        slower_and_sooner.death_at = Some(1.0);
        fast.death_at = Some(2.0);
        assert_eq!(
            compare_success(&fast, &slower_and_sooner),
            Ordering::Greater
        );
    }

    #[test]
    fn target_commitment_ignores_small_score_noise_but_allows_material_gain() {
        let candidate = |score, utility, continuity| Evaluation {
            angle: 0.0,
            goal: super::Goal::CollectBounties,
            total_potential_score: score,
            route_score: 0.0,
            time_to_last_bounty: None,
            score_rate: 1.0,
            first_bounty_rate: 0.0,
            time_to_reach_score: 5.0,
            selection_utility: utility,
            average_speed_ratio: 1.0,
            terminal_speed_ratio: 1.0,
            bounty_count: 1,
            continuity,
            keeps_target: true,
            target: None,
            death_at: None,
            death_reason: None,
            risk_exposure: 0.0,
        };
        let held = candidate(100.0, 104.0, 1.0);
        let small_gain = candidate(104.0, 102.0, 0.0);
        let meaningful_gain = candidate(120.0, 118.0, 0.0);
        assert_eq!(compare_success(&held, &small_gain), Ordering::Greater);
        assert_eq!(compare_success(&meaningful_gain, &held), Ordering::Greater);
    }

    #[test]
    fn safe_route_utility_keeps_priority_over_short_window_score_rate_spikes() {
        let route = |utility, rate| Evaluation {
            angle: 0.0,
            goal: super::Goal::CollectBounties,
            total_potential_score: utility,
            route_score: 100.0,
            time_to_last_bounty: Some(1.0),
            score_rate: rate,
            first_bounty_rate: rate,
            time_to_reach_score: 5.0,
            selection_utility: utility,
            average_speed_ratio: 1.0,
            terminal_speed_ratio: 1.0,
            bounty_count: 1,
            continuity: 0.0,
            keeps_target: false,
            target: None,
            death_at: None,
            death_reason: None,
            risk_exposure: 0.0,
        };
        let sustained_route = route(120.0, 10.0);
        let short_spike = route(110.0, 100.0);
        assert_eq!(
            compare_success(&sustained_route, &short_spike),
            Ordering::Greater
        );
    }

    #[test]
    fn agile_top1_ranks_safe_routes_by_score_rate_but_never_over_death() {
        let route = |rate: f64, death_at: Option<f64>| Evaluation {
            angle: 0.0,
            goal: if death_at.is_some() {
                super::Goal::FastestDeath
            } else {
                super::Goal::CollectBounties
            },
            total_potential_score: 100.0,
            route_score: rate * 2.0,
            time_to_last_bounty: Some(2.0),
            score_rate: rate,
            first_bounty_rate: rate,
            time_to_reach_score: 1.0,
            selection_utility: 100.0,
            average_speed_ratio: 1.0,
            terminal_speed_ratio: 1.0,
            bounty_count: usize::from(rate > 0.0),
            continuity: 0.0,
            keeps_target: false,
            target: None,
            death_at,
            death_reason: death_at.map(|_| DeathReason::Anomaly),
            risk_exposure: 0.0,
        };
        let top = route(20.0, None);
        let lower_rate = route(10.0, None);
        let safe_empty = route(0.0, None);
        let fatal = route(10_000.0, Some(1.0));
        assert_eq!(compare_agile_top1(&top, &lower_rate), Ordering::Greater);
        assert_eq!(compare_agile_top1(&safe_empty, &fatal), Ordering::Greater);
    }

    #[test]
    fn profitable_course_switch_requires_material_gain_but_zero_may_switch() {
        assert_eq!(score_rate(120.0, Some(12.0)), 10.0);
        assert_eq!(score_rate(-5.0, Some(12.0)), 0.0);
        assert_eq!(score_rate(120.0, None), 0.0);
        assert_eq!(score_rate(120.0, Some(0.1)), 600.0);
        assert!(!materially_more_profitable(10.0, 14.99));
        assert!(materially_more_profitable(10.0, 15.0));
        assert!(!materially_more_profitable(0.0, 4.99));
        assert!(materially_more_profitable(0.0, 5.0));
        assert!(!materially_more_profitable(0.0, 0.0));
    }

    #[test]
    fn minor_speed_noise_does_not_break_course_continuity() {
        let candidate = |speed, utility, continuity| Evaluation {
            angle: 0.0,
            goal: super::Goal::CollectBounties,
            total_potential_score: utility,
            route_score: 0.0,
            time_to_last_bounty: None,
            score_rate: 1.0,
            first_bounty_rate: 0.0,
            time_to_reach_score: 3.0,
            selection_utility: utility,
            average_speed_ratio: speed,
            terminal_speed_ratio: speed,
            bounty_count: 1,
            continuity,
            keeps_target: false,
            target: None,
            death_at: None,
            death_reason: None,
            risk_exposure: 0.0,
        };
        let steady = candidate(0.990, 50.0, 1.0);
        let twitch = candidate(1.000, 49.0, 0.0);
        assert_eq!(compare_success(&steady, &twitch), Ordering::Greater);

        let materially_faster = candidate(0.980, 1.0, 1.0);
        assert_eq!(
            compare_success(&twitch, &materially_faster),
            Ordering::Greater
        );
    }

    #[test]
    fn evaluation_keeps_reachable_target_and_reports_score_timing() {
        let target = Bounty {
            position: Vec2 { x: 150.0, y: 100.0 },
            points: 100.0,
            radius: 5.0,
        };
        let transport = Transport {
            id: "carpet".into(),
            position: Vec2 { x: 100.0, y: 100.0 },
            velocity: Vec2 { x: 30.0, y: 0.0 },
            self_acceleration: Vec2::default(),
            anomaly_acceleration: Vec2::default(),
            alive: true,
        };
        let desert = Desert {
            transports: vec![transport.clone()],
            bounties: vec![
                target.clone(),
                Bounty {
                    position: Vec2 { x: 250.0, y: 100.0 },
                    points: 200.0,
                    radius: 5.0,
                },
            ],
            ..empty_desert()
        };
        let path = evaluate(
            &desert,
            &transport,
            0.0,
            0.3,
            4.0,
            &desert.bounties,
            &[],
            Some(&target),
            None,
            true,
            false,
        );
        assert!(path.keeps_target);
        assert!(path.total_potential_score > 0.0);
        assert_eq!(path.bounty_count, 2);
        assert_eq!(path.route_score, 300.0);
        let last_coin_time = path.time_to_last_bounty.unwrap();
        assert!(last_coin_time > path.time_to_reach_score);
        assert_eq!(path.score_rate, path.route_score / last_coin_time);
        assert_eq!(
            path.first_bounty_rate,
            target.points / path.time_to_reach_score
        );
        assert!(path.time_to_reach_score >= 0.3);
        assert!(path.average_speed_ratio < path.terminal_speed_ratio);
        assert!(path.terminal_speed_ratio > 0.99);
        assert!(path
            .target
            .is_some_and(|selected| same_bounty(&selected, &target)));
    }

    #[test]
    fn last_bounty_time_is_max_event_time_not_grid_iteration_order() {
        let mut desert = empty_desert();
        desert.map_size = Vec2 {
            x: 1000.0,
            y: 1000.0,
        };
        desert.max_speed = 100.0;
        desert.max_accel = 0.0;
        desert.transport_radius = 1.0;
        desert.bounties = vec![
            Bounty {
                position: Vec2 { x: 148.0, y: 500.0 },
                points: 20.0,
                radius: 1.0,
            },
            Bounty {
                position: Vec2 { x: 145.0, y: 500.0 },
                points: 10.0,
                radius: 1.0,
            },
        ];
        let transport = Transport {
            id: "carpet".into(),
            position: Vec2 { x: 100.0, y: 500.0 },
            velocity: Vec2 { x: 100.0, y: 0.0 },
            self_acceleration: Vec2::default(),
            anomaly_acceleration: Vec2::default(),
            alive: true,
        };
        let grid = super::BountyGrid::new(&desert.bounties, desert.transport_radius);
        let path = evaluate(
            &desert,
            &transport,
            0.0,
            0.3,
            0.4,
            &desert.bounties,
            &[],
            None,
            Some(&grid),
            false,
            false,
        );
        assert_eq!(path.bounty_count, 2);
        assert!(
            path.time_to_last_bounty.unwrap() > path.time_to_reach_score,
            "first={}, last={:?}",
            path.time_to_reach_score,
            path.time_to_last_bounty
        );
    }

    #[test]
    fn near_miss_bonus_does_not_count_as_a_collected_bounty() {
        let target = Bounty {
            position: Vec2 { x: 180.0, y: 100.0 },
            points: 100.0,
            radius: 5.0,
        };
        let transport = Transport {
            id: "carpet".into(),
            position: Vec2 { x: 100.0, y: 114.0 },
            velocity: Vec2 { x: 30.0, y: 0.0 },
            self_acceleration: Vec2::default(),
            anomaly_acceleration: Vec2::default(),
            alive: true,
        };
        let desert = Desert {
            transports: vec![transport.clone()],
            bounties: vec![target],
            ..empty_desert()
        };
        let path = evaluate(
            &desert,
            &transport,
            0.0,
            0.3,
            2.0,
            &desert.bounties,
            &[],
            None,
            None,
            true,
            false,
        );
        assert_eq!(path.bounty_count, 0);
        assert_eq!(path.goal, super::Goal::AvoidDeath);
        assert!(path.total_potential_score > 0.0);
        assert_eq!(path.route_score, 0.0);
        assert_eq!(path.time_to_last_bounty, None);
        assert_eq!(path.score_rate, 0.0);
    }

    #[test]
    fn planner_holds_course_under_marginal_noise_and_reports_profit_rate() {
        let target = Bounty {
            position: Vec2 { x: 500.0, y: 100.0 },
            points: 100.0,
            radius: 5.0,
        };
        let transport = Transport {
            id: "carpet".into(),
            position: Vec2 { x: 100.0, y: 100.0 },
            velocity: Vec2 { x: 30.0, y: 0.0 },
            self_acceleration: Vec2::default(),
            anomaly_acceleration: Vec2::default(),
            alive: true,
        };
        let mut desert = Desert {
            transports: vec![transport],
            bounties: vec![target.clone()],
            ..empty_desert()
        };
        // The target is deliberately beyond the former 3-second commitment window.
        let mut bot = Bot::new(6.0);
        bot.plan(&desert);
        let initial_angle = bot.plans["carpet"].angle.unwrap();
        let selected_before = bot.plans["carpet"].target.clone().unwrap();
        assert!(same_bounty(&selected_before, &target));

        desert.bounties.push(Bounty {
            position: Vec2 { x: 100.0, y: 400.0 },
            points: 110.0,
            radius: 5.0,
        });
        // Advance one server tick so the committed target ETA can show expected progress.
        desert.transports[0].position.x += 6.0;
        let marginal_plan = bot.plan(&desert);
        assert!(marginal_plan.trajectory_evaluations >= 36);
        let selected_after = bot.plans["carpet"].target.as_ref().unwrap();
        assert!(same_bounty(selected_after, &target));
        assert!(
            super::angular_delta(bot.plans["carpet"].angle.unwrap(), initial_angle).abs() < 1e-9
        );
        assert!(marginal_plan.carpets[0].score_rate.unwrap() > 0.0);

        let lucrative = Bounty {
            position: Vec2 { x: 340.0, y: 300.0 },
            points: 10_000.0,
            radius: 5.0,
        };
        desert.bounties.push(lucrative.clone());
        let held_plan = bot.plan(&desert);
        assert!(held_plan.trajectory_evaluations >= 36);
        assert!(
            super::angular_delta(bot.plans["carpet"].angle.unwrap(), initial_angle).abs() < 1e-9
        );
        assert!(same_bounty(
            bot.plans["carpet"].target.as_ref().unwrap(),
            &target
        ));

        // Once the snapshot confirms the first coin was collected, release the
        // committed course and let the global scan choose the newly valuable route.
        desert.transports[0].position = Vec2 { x: 510.0, y: 100.0 };
        desert.transports[0].velocity = Vec2 { x: 100.0, y: 0.0 };
        desert
            .bounties
            .retain(|bounty| !same_bounty(bounty, &target));
        let released_plan = bot.plan(&desert);
        assert!(released_plan.trajectory_evaluations > 1);
        assert!(bot.plans["carpet"]
            .target
            .as_ref()
            .is_none_or(|selected| !same_bounty(selected, &target)));
    }

    #[test]
    fn agile_top1_has_a_fifteen_second_cap_and_never_persists_plan_memo() {
        let mut bot = Bot::with_strategy(30.0, Strategy::AgileTop1);
        assert_eq!(bot.strategy, Strategy::AgileTop1);
        assert_eq!(bot.horizon, 15.0);
        let mut desert = empty_desert();
        desert.transports.push(Transport {
            id: "agile".into(),
            position: Vec2 { x: 800.0, y: 800.0 },
            velocity: Vec2 { x: 40.0, y: 0.0 },
            self_acceleration: Vec2::default(),
            anomaly_acceleration: Vec2::default(),
            alive: true,
        });
        let first = bot.plan(&desert);
        assert_eq!(first.trajectory_evaluations, 360);
        assert!(bot.plans.is_empty());
        let second = bot.plan(&desert);
        assert_eq!(second.trajectory_evaluations, 360);
        assert!(bot.plans.is_empty());
        assert_eq!(bot.horizon, 15.0);
    }

    #[test]
    fn strategy_names_are_explicit_and_unknown_values_are_rejected() {
        assert_eq!(
            Strategy::parse("stable-profit").unwrap(),
            Strategy::StableProfit
        );
        assert_eq!(Strategy::parse("agile-top1").unwrap(), Strategy::AgileTop1);
        assert!(Strategy::parse("typo").is_err());
        assert_eq!(Strategy::default(), Strategy::StableProfit);
        assert_eq!(Bot::new(10.0).strategy, Strategy::StableProfit);
        assert_eq!(Bot::with_strategy(8.0, Strategy::AgileTop1).horizon, 8.0);
    }

    #[test]
    fn movement_and_doom_modes_are_independently_configurable() {
        assert_eq!(
            MovementStrategy::parse("survival").unwrap(),
            MovementStrategy::Survival
        );
        assert_eq!(
            MovementStrategy::parse("none").unwrap(),
            MovementStrategy::None
        );
        assert!(MovementStrategy::parse("typo").is_err());
        assert_eq!(DoomPolicy::parse("collect").unwrap(), DoomPolicy::Collect);
        assert_eq!(DoomPolicy::parse("fastest").unwrap(), DoomPolicy::Fastest);
        assert!(DoomPolicy::parse("typo").is_err());
    }

    #[test]
    fn death_risk_is_low_in_clear_space_and_saturates_in_lethal_core() {
        let mut desert = empty_desert();
        let point = Vec2 { x: 525.0, y: 525.0 };
        assert_eq!(
            estimated_death_risk(point, Vec2::default(), 0.0, &desert),
            0.0
        );
        desert.anomalies.push(Anomaly {
            position: point,
            velocity: Vec2::default(),
            core_radius: 30.0,
            effect_radius: 200.0,
            strength: 20.0,
        });
        assert_eq!(
            estimated_death_risk(point, Vec2::default(), 0.0, &desert),
            1.0
        );
        let outside =
            estimated_death_risk(Vec2 { x: 900.0, y: 900.0 }, Vec2::default(), 0.0, &desert);
        assert!(
            outside < 0.1,
            "clear position should have low risk: {outside}"
        );
    }

    #[test]
    fn survival_waits_for_imminent_death_or_scarce_escape_routes() {
        assert_eq!(survival_escape_trigger(None, 0, 72), None);
        assert_eq!(
            survival_escape_trigger(Some(2.9), 70, 72),
            Some(EscapeTrigger::Imminent)
        );
        assert_eq!(
            survival_escape_trigger(Some(3.0), 70, 72),
            Some(EscapeTrigger::Imminent)
        );
        assert_eq!(survival_escape_trigger(Some(3.01), 70, 72), None);
        assert_eq!(
            survival_escape_trigger(Some(20.0), 7, 72),
            Some(EscapeTrigger::ScarceOptions)
        );
        assert_eq!(survival_escape_trigger(Some(20.0), 8, 72), None);
        assert_eq!(survival_escape_trigger(Some(20.0), 70, 72), None);
    }

    #[test]
    fn risk_grid_marks_uncompensable_force_chain_into_anomaly_core_fatal() {
        let mut desert = empty_desert();
        desert.max_accel = 40.0;
        desert.anomalies.push(Anomaly {
            position: Vec2 { x: 500.0, y: 525.0 },
            velocity: Vec2::default(),
            core_radius: 10.0,
            effect_radius: 400.0,
            strength: 80.0,
        });
        // The sampled cell center is x=175; field force cannot be resisted and
        // continues along the grid to the anomaly's lethal core at x=525.
        assert_eq!(
            estimated_death_risk(Vec2 { x: 200.0, y: 525.0 }, Vec2::default(), 0.0, &desert),
            1.0
        );
    }
}

use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AdaptiveDecision {
    Maintain,
    Increase { from: usize, to: usize },
    Decrease { from: usize, to: usize },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AdaptiveSample {
    pub bytes: u64,
    pub elapsed: Duration,
    pub active_concurrency: usize,
    pub remaining_segments: usize,
    pub completed_segments: usize,
    pub request_failures: u8,
    pub cancelled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DecreaseReason {
    ThroughputRegression,
    RequestFailures,
    UnproductiveProbe,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdaptiveConfig {
    pub enabled: bool,
    pub min_concurrency: usize,
    pub initial_concurrency: usize,
    /// A hard ceiling, not a guaranteed number of active requests. Values above 32 are rejected.
    pub max_concurrency: usize,
    pub measurement_window: Duration,
    pub improvement_threshold_percent: u8,
    pub regression_threshold_percent: u8,
    pub observations_to_increase: u8,
    pub cooldown_windows: u8,
    pub failures_to_decrease: u8,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_concurrency: 1,
            initial_concurrency: 4,
            max_concurrency: 8,
            measurement_window: Duration::from_millis(500),
            improvement_threshold_percent: 10,
            regression_threshold_percent: 20,
            observations_to_increase: 2,
            cooldown_windows: 2,
            failures_to_decrease: 2,
        }
    }
}

impl AdaptiveConfig {
    pub(super) fn is_valid(&self) -> bool {
        self.min_concurrency >= 1
            && self.min_concurrency <= self.initial_concurrency
            && self.initial_concurrency <= self.max_concurrency
            && self.max_concurrency <= 32
            && self.measurement_window >= Duration::from_millis(10)
            && (1..=100).contains(&self.improvement_threshold_percent)
            && (1..=100).contains(&self.regression_threshold_percent)
            && self.observations_to_increase >= 2
            && self.cooldown_windows > 0
            && self.failures_to_decrease > 0
    }
}

struct Probe {
    baseline_bytes_per_second: u64,
    positive_observations: u8,
    observations: u8,
}

pub(super) struct AdaptiveController {
    config: AdaptiveConfig,
    desired_concurrency: usize,
    baseline_bytes_per_second: Option<u64>,
    smoothed_bytes_per_second: Option<u64>,
    probe: Option<Probe>,
    probe_blocked: bool,
    healthy_observations: u8,
    regression_observations: u8,
    cooldown_remaining: u8,
    failures: u8,
}

impl AdaptiveController {
    pub fn new(config: AdaptiveConfig) -> Self {
        Self {
            desired_concurrency: config.initial_concurrency,
            config,
            baseline_bytes_per_second: None,
            smoothed_bytes_per_second: None,
            probe: None,
            probe_blocked: false,
            healthy_observations: 0,
            regression_observations: 0,
            cooldown_remaining: 0,
            failures: 0,
        }
    }

    pub fn desired_concurrency(&self, remaining_segments: usize) -> usize {
        self.desired_concurrency.min(remaining_segments)
    }

    pub fn observe(
        &mut self,
        sample: AdaptiveSample,
    ) -> (AdaptiveDecision, Option<DecreaseReason>) {
        if sample.cancelled || !self.config.enabled {
            return (AdaptiveDecision::Maintain, None);
        }

        self.failures = self.failures.saturating_add(sample.request_failures);
        let bytes_per_second = throughput(sample.bytes, sample.elapsed);
        let smoothed = self.smooth(bytes_per_second);

        if self.failures >= self.config.failures_to_decrease {
            self.failures = 0;
            self.healthy_observations = 0;
            self.regression_observations = 0;
            self.probe = None;
            self.baseline_bytes_per_second = Some(smoothed);
            self.cooldown_remaining = self.config.cooldown_windows;
            return (self.decrease(), Some(DecreaseReason::RequestFailures));
        }

        if sample.bytes == 0 || sample.elapsed.is_zero() {
            return (AdaptiveDecision::Maintain, None);
        }

        if let Some(probe) = self.probe.as_mut() {
            if sample.active_concurrency < self.desired_concurrency {
                return (AdaptiveDecision::Maintain, None);
            }

            probe.observations = probe.observations.saturating_add(1);
            if meets_improvement(
                smoothed,
                probe.baseline_bytes_per_second,
                self.config.improvement_threshold_percent,
            ) {
                probe.positive_observations = probe.positive_observations.saturating_add(1);
                if probe.positive_observations >= self.config.observations_to_increase {
                    self.probe = None;
                    self.baseline_bytes_per_second = Some(smoothed);
                    self.healthy_observations = 0;
                    self.cooldown_remaining = self.config.cooldown_windows;
                    return (AdaptiveDecision::Maintain, None);
                }
            }

            if probe.observations >= self.config.observations_to_increase {
                let baseline = probe.baseline_bytes_per_second;
                self.probe = None;
                self.baseline_bytes_per_second = Some(baseline);
                self.probe_blocked = true;
                self.cooldown_remaining = self.config.cooldown_windows;
                return (self.decrease(), Some(DecreaseReason::UnproductiveProbe));
            }
            return (AdaptiveDecision::Maintain, None);
        }

        if self.cooldown_remaining > 0 {
            self.cooldown_remaining -= 1;
            return (AdaptiveDecision::Maintain, None);
        }

        let Some(baseline) = self.baseline_bytes_per_second else {
            self.baseline_bytes_per_second = Some(smoothed);
            self.healthy_observations = 1;
            return (AdaptiveDecision::Maintain, None);
        };

        if meets_regression(smoothed, baseline, self.config.regression_threshold_percent) {
            self.regression_observations = self.regression_observations.saturating_add(1);
            if self.regression_observations >= self.config.observations_to_increase {
                self.baseline_bytes_per_second = Some(smoothed);
                self.regression_observations = 0; // reset so cooldown period doesn't immediately re-trigger
                self.healthy_observations = 0;
                self.probe_blocked = true;
                self.cooldown_remaining = self.config.cooldown_windows;
                return (self.decrease(), Some(DecreaseReason::ThroughputRegression));
            }
            return (AdaptiveDecision::Maintain, None);
        }

        self.regression_observations = 0;
        if self.probe_blocked {
            if !meets_improvement(
                smoothed,
                baseline,
                self.config.improvement_threshold_percent,
            ) {
                self.healthy_observations = 0;
                return (AdaptiveDecision::Maintain, None);
            }
            self.probe_blocked = false;
            self.baseline_bytes_per_second = Some(smoothed);
            self.healthy_observations = 1;
            return (AdaptiveDecision::Maintain, None);
        }
        if smoothed > baseline {
            self.baseline_bytes_per_second = Some(smoothed);
        }
        self.healthy_observations = self.healthy_observations.saturating_add(1);
        if self.healthy_observations < self.config.observations_to_increase
            || self.desired_concurrency >= self.config.max_concurrency
            || sample.remaining_segments <= self.desired_concurrency
            || sample.active_concurrency != self.desired_concurrency
            || sample.completed_segments == 0
        {
            return (AdaptiveDecision::Maintain, None);
        }

        let from = self.desired_concurrency;
        let to = from + 1;
        self.desired_concurrency = to;
        self.probe = Some(Probe {
            baseline_bytes_per_second: baseline,
            positive_observations: 0,
            observations: 0,
        });
        self.healthy_observations = 0;
        (AdaptiveDecision::Increase { from, to }, None)
    }

    fn smooth(&mut self, sample_bytes_per_second: u64) -> u64 {
        let Some(previous) = self.smoothed_bytes_per_second else {
            self.smoothed_bytes_per_second = Some(sample_bytes_per_second);
            return sample_bytes_per_second;
        };
        let smoothed = (u128::from(previous) + u128::from(sample_bytes_per_second)) / 2;
        let smoothed = u64::try_from(smoothed).unwrap_or(u64::MAX);
        self.smoothed_bytes_per_second = Some(smoothed);
        smoothed
    }

    fn decrease(&mut self) -> AdaptiveDecision {
        let from = self.desired_concurrency;
        if from <= self.config.min_concurrency {
            return AdaptiveDecision::Maintain;
        }
        self.desired_concurrency -= 1;
        AdaptiveDecision::Decrease {
            from,
            to: self.desired_concurrency,
        }
    }
}

fn throughput(bytes: u64, elapsed: Duration) -> u64 {
    if elapsed.is_zero() {
        return 0;
    }
    let rate = u128::from(bytes).saturating_mul(1_000_000_000) / elapsed.as_nanos();
    u64::try_from(rate).unwrap_or(u64::MAX)
}

fn meets_improvement(value: u64, baseline: u64, threshold_percent: u8) -> bool {
    u128::from(value) * 100 >= u128::from(baseline) * (100 + u128::from(threshold_percent))
}

fn meets_regression(value: u64, baseline: u64, threshold_percent: u8) -> bool {
    u128::from(value) * 100 <= u128::from(baseline) * (100 - u128::from(threshold_percent))
}

#[cfg(test)]
mod tests {
    use super::{
        AdaptiveConfig, AdaptiveController, AdaptiveDecision, AdaptiveSample, DecreaseReason,
    };
    use std::time::Duration;

    fn sample(bytes: u64, active: usize, remaining: usize) -> AdaptiveSample {
        AdaptiveSample {
            bytes,
            elapsed: Duration::from_secs(1),
            active_concurrency: active,
            remaining_segments: remaining,
            completed_segments: 1,
            request_failures: 0,
            cancelled: false,
        }
    }

    fn config() -> AdaptiveConfig {
        AdaptiveConfig {
            measurement_window: Duration::from_secs(1),
            cooldown_windows: 2,
            ..AdaptiveConfig::default()
        }
    }

    fn establish_baseline(controller: &mut AdaptiveController) {
        assert_eq!(
            controller.observe(sample(100, 4, 8)).0,
            AdaptiveDecision::Maintain
        );
        assert_eq!(
            controller.observe(sample(100, 4, 8)).0,
            AdaptiveDecision::Increase { from: 4, to: 5 }
        );
    }

    #[test]
    fn starts_at_configured_initial_concurrency() {
        let controller = AdaptiveController::new(config());
        assert_eq!(controller.desired_concurrency(8), 4);
    }

    #[test]
    fn empty_windows_do_not_count_as_throughput_evidence() {
        let mut controller = AdaptiveController::new(config());
        controller.observe(sample(0, 4, 8));
        controller.observe(sample(0, 4, 8));
        assert_eq!(
            controller.observe(sample(100, 4, 8)).0,
            AdaptiveDecision::Maintain
        );
        assert_eq!(
            controller.observe(sample(100, 4, 8)).0,
            AdaptiveDecision::Increase { from: 4, to: 5 }
        );
    }

    #[test]
    fn healthy_improvement_is_confirmed_before_committing_a_probe() {
        let mut controller = AdaptiveController::new(config());
        establish_baseline(&mut controller);
        assert_eq!(
            controller.observe(sample(120, 5, 8)).0,
            AdaptiveDecision::Maintain
        );
        assert_eq!(
            controller.observe(sample(120, 5, 8)).0,
            AdaptiveDecision::Maintain
        );
        assert_eq!(controller.desired_concurrency(8), 5);
    }

    #[test]
    fn improvement_below_threshold_rolls_back_the_probe() {
        let mut controller = AdaptiveController::new(config());
        establish_baseline(&mut controller);
        controller.observe(sample(105, 5, 8));
        let decision = controller.observe(sample(105, 5, 8));
        assert_eq!(decision.0, AdaptiveDecision::Decrease { from: 5, to: 4 });
        assert_eq!(decision.1, Some(DecreaseReason::UnproductiveProbe));
    }

    #[test]
    fn repeated_positive_observations_are_required_for_each_probe() {
        let mut controller = AdaptiveController::new(config());
        establish_baseline(&mut controller);
        controller.observe(sample(125, 5, 8));
        controller.observe(sample(125, 5, 8));
        for _ in 0..2 {
            controller.observe(sample(125, 5, 8));
        }
        controller.observe(sample(125, 5, 8));
        let (decision, _) = controller.observe(sample(125, 5, 8));
        assert_eq!(decision, AdaptiveDecision::Increase { from: 5, to: 6 });
    }

    #[test]
    fn maximum_and_minimum_are_respected() {
        let mut upper = AdaptiveConfig {
            initial_concurrency: 7,
            max_concurrency: 8,
            observations_to_increase: 2,
            cooldown_windows: 1,
            ..config()
        };
        let mut controller = AdaptiveController::new(upper.clone());
        controller.observe(sample(100, 7, 8));
        assert_eq!(
            controller.observe(sample(100, 7, 8)).0,
            AdaptiveDecision::Increase { from: 7, to: 8 }
        );
        controller.observe(sample(200, 8, 8));
        assert_eq!(
            controller.observe(sample(200, 8, 8)).0,
            AdaptiveDecision::Maintain
        );

        upper.min_concurrency = 4;
        upper.initial_concurrency = 4;
        upper.max_concurrency = 4;
        let mut lower = AdaptiveController::new(upper);
        lower.observe(sample(100, 4, 8));
        lower.observe(sample(100, 4, 8));
        let (decision, reason) = lower.observe(AdaptiveSample {
            request_failures: 2,
            ..sample(0, 4, 8)
        });
        assert_eq!(decision, AdaptiveDecision::Maintain);
        assert_eq!(reason, Some(DecreaseReason::RequestFailures));
        assert_eq!(lower.desired_concurrency(8), 4);
    }

    #[test]
    fn configured_extreme_ceiling_is_bounded_at_thirty_two_workers() {
        let extreme = AdaptiveConfig {
            min_concurrency: 8,
            initial_concurrency: 8,
            max_concurrency: 32,
            ..config()
        };
        assert!(extreme.is_valid());
        assert_eq!(AdaptiveController::new(extreme).desired_concurrency(64), 8);
        assert!(
            !AdaptiveConfig {
                max_concurrency: 33,
                ..config()
            }
            .is_valid()
        );
    }

    #[test]
    fn balanced_turbo_and_extreme_ceiling_configurations_are_accepted() {
        for (minimum, initial, maximum) in [(2, 2, 8), (4, 4, 16), (8, 8, 32)] {
            let profile = AdaptiveConfig {
                min_concurrency: minimum,
                initial_concurrency: initial,
                max_concurrency: maximum,
                ..config()
            };
            assert!(profile.is_valid());
            assert_eq!(
                AdaptiveController::new(profile).desired_concurrency(maximum),
                initial
            );
        }
    }

    #[test]
    fn sustained_throughput_regression_decreases_concurrency() {
        let mut controller = AdaptiveController::new(config());
        establish_baseline(&mut controller);
        controller.observe(sample(125, 5, 8));
        controller.observe(sample(125, 5, 8));
        controller.observe(sample(125, 5, 8));
        controller.observe(sample(125, 5, 8));
        controller.observe(sample(125, 5, 8));
        controller.observe(sample(40, 5, 8));
        let (decision, reason) = controller.observe(sample(40, 5, 8));
        assert_eq!(decision, AdaptiveDecision::Decrease { from: 5, to: 4 });
        assert_eq!(reason, Some(DecreaseReason::ThroughputRegression));
    }

    #[test]
    fn repeated_request_failures_reduce_concurrency_and_cool_down() {
        let mut controller = AdaptiveController::new(config());
        establish_baseline(&mut controller);
        let (decision, reason) = controller.observe(AdaptiveSample {
            request_failures: 2,
            ..sample(100, 5, 8)
        });
        assert_eq!(decision, AdaptiveDecision::Decrease { from: 5, to: 4 });
        assert_eq!(reason, Some(DecreaseReason::RequestFailures));
        assert_eq!(
            controller.observe(sample(100, 4, 8)).0,
            AdaptiveDecision::Maintain
        );
        assert_eq!(
            controller.observe(sample(100, 4, 8)).0,
            AdaptiveDecision::Maintain
        );
        assert_eq!(controller.desired_concurrency(8), 4);
    }

    #[test]
    fn cooldown_prevents_immediate_repeated_changes() {
        let mut controller = AdaptiveController::new(config());
        establish_baseline(&mut controller);
        controller.observe(sample(125, 5, 8));
        controller.observe(sample(125, 5, 8));
        for _ in 0..2 {
            controller.observe(sample(125, 5, 8));
        }
        controller.observe(sample(125, 5, 8));
        assert_eq!(
            controller.observe(sample(125, 5, 8)).0,
            AdaptiveDecision::Increase { from: 5, to: 6 }
        );
        assert_eq!(controller.desired_concurrency(8), 6);
    }

    #[test]
    fn rejected_probe_waits_for_measured_improvement_before_trying_again() {
        let mut controller = AdaptiveController::new(config());
        establish_baseline(&mut controller);
        controller.observe(sample(100, 5, 8));
        assert_eq!(
            controller.observe(sample(100, 5, 8)).0,
            AdaptiveDecision::Decrease { from: 5, to: 4 }
        );
        for _ in 0..8 {
            assert_eq!(
                controller.observe(sample(100, 4, 8)).0,
                AdaptiveDecision::Maintain
            );
        }
        assert_eq!(controller.desired_concurrency(8), 4);
        controller.observe(sample(500, 4, 8));
        assert_eq!(
            controller.observe(sample(500, 4, 8)).0,
            AdaptiveDecision::Increase { from: 4, to: 5 }
        );
    }

    #[test]
    fn remaining_segments_limit_effective_concurrency_and_probing() {
        let mut controller = AdaptiveController::new(config());
        assert_eq!(controller.desired_concurrency(2), 2);
        controller.observe(sample(100, 4, 4));
        assert_eq!(
            controller.observe(sample(100, 4, 4)).0,
            AdaptiveDecision::Maintain
        );
        assert_eq!(controller.desired_concurrency(4), 4);
    }

    #[test]
    fn disabled_adaptation_keeps_the_initial_concurrency() {
        let mut config = config();
        config.enabled = false;
        let mut controller = AdaptiveController::new(config);
        for _ in 0..10 {
            controller.observe(sample(1_000_000, 4, 8));
        }
        assert_eq!(controller.desired_concurrency(8), 4);
    }

    #[test]
    fn invalid_configuration_is_rejected() {
        let invalid = [
            AdaptiveConfig {
                min_concurrency: 0,
                ..config()
            },
            AdaptiveConfig {
                initial_concurrency: 9,
                ..config()
            },
            AdaptiveConfig {
                max_concurrency: 33,
                ..config()
            },
            AdaptiveConfig {
                improvement_threshold_percent: 0,
                ..config()
            },
            AdaptiveConfig {
                observations_to_increase: 1,
                ..config()
            },
            AdaptiveConfig {
                measurement_window: Duration::ZERO,
                ..config()
            },
            AdaptiveConfig {
                measurement_window: Duration::from_millis(1),
                ..config()
            },
        ];
        assert!(invalid.into_iter().all(|config| !config.is_valid()));
        assert!(config().is_valid());
    }

    #[test]
    fn controller_decisions_are_deterministic() {
        let mut first = AdaptiveController::new(config());
        let mut second = AdaptiveController::new(config());
        let samples = [
            sample(100, 4, 8),
            sample(100, 4, 8),
            sample(130, 5, 8),
            sample(130, 5, 8),
        ];
        let first_decisions = samples
            .iter()
            .map(|sample| first.observe(*sample))
            .collect::<Vec<_>>();
        let second_decisions = samples
            .iter()
            .map(|sample| second.observe(*sample))
            .collect::<Vec<_>>();
        assert_eq!(first_decisions, second_decisions);
    }

    #[test]
    fn cancellation_does_not_trigger_adaptation() {
        let mut controller = AdaptiveController::new(config());
        let cancelled = AdaptiveSample {
            cancelled: true,
            ..sample(u64::MAX, 4, 8)
        };
        assert_eq!(
            controller.observe(cancelled),
            (AdaptiveDecision::Maintain, None)
        );
        assert_eq!(controller.desired_concurrency(8), 4);
    }
}

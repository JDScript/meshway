use super::*;

fn observation(value: usize) -> RateKey {
    RateKey(
        format!("00000000-0000-4000-8000-{value:012}")
            .parse()
            .unwrap(),
        [0; 32],
    )
}

#[test]
fn source_rate_remains_strict_and_rejections_do_not_extend_its_window() {
    let mut probes = Probes::default();
    let id = observation(1);
    let now = Instant::now();
    probes.admit_rate(id, RateRole::Source, now).unwrap();
    for millis in [0, 110, 500, 999] {
        assert_eq!(
            probes.admit_rate(id, RateRole::Source, now + Duration::from_millis(millis)),
            Err(Error::Capacity)
        );
        assert_eq!(probes.rates[&id].full_at, now + INTERVAL);
    }
    probes
        .admit_rate(id, RateRole::Source, now + INTERVAL)
        .unwrap();
    assert_eq!(probes.rates[&id].full_at, now + INTERVAL * 2);
    assert_eq!(
        probes.admit_rate(id, RateRole::Remote, now + INTERVAL),
        Err(Error::Policy)
    );
}

#[test]
fn remote_bucket_accepts_compressed_arrivals_and_refills_at_one_per_second() {
    let mut probes = Probes::default();
    let id = observation(1);
    let now = Instant::now();
    // Source emits at 0 and 1000 ms; path delays are 900 and 10 ms.
    probes
        .admit_rate(id, RateRole::Remote, now + Duration::from_millis(900))
        .unwrap();
    probes
        .admit_rate(id, RateRole::Remote, now + Duration::from_millis(1010))
        .unwrap();
    let full_at = now + Duration::from_millis(2900);
    assert_eq!(probes.rates[&id].full_at, full_at);
    for millis in 1010..1900 {
        let at = now + Duration::from_millis(millis);
        probes.maintain_rates(at);
        assert_eq!(
            probes.admit_rate(id, RateRole::Remote, at),
            Err(Error::Capacity)
        );
        assert_eq!(probes.rates[&id].full_at, full_at);
    }
    probes
        .admit_rate(id, RateRole::Remote, now + Duration::from_millis(1900))
        .unwrap();
    assert_eq!(probes.rates[&id].full_at, now + Duration::from_millis(3900));
}

#[test]
fn remote_continuous_burst_is_bounded_without_polling_or_idle_credit_accumulation() {
    let mut probes = Probes::default();
    let id = observation(1);
    let now = Instant::now();
    let mut admitted = 0;
    for millis in 0..=10_000 {
        let at = now + Duration::from_millis(millis);
        for _ in 0..4 {
            probes.maintain_rates(at);
            match probes.admit_rate(id, RateRole::Remote, at) {
                Ok(()) => admitted += 1,
                Err(Error::Capacity) => {}
                _ => panic!("unexpected rate result"),
            }
        }
        assert!(admitted <= 2 + millis / 1000);
    }
    assert_eq!(admitted, 12);
    let later = now + Duration::from_secs(100);
    probes.maintain_rates(later);
    assert!(probes.rates.is_empty());
    probes.admit_rate(id, RateRole::Remote, later).unwrap();
    probes.admit_rate(id, RateRole::Remote, later).unwrap();
    assert_eq!(
        probes.admit_rate(id, RateRole::Remote, later),
        Err(Error::Capacity)
    );
}

#[test]
fn rate_map_capacity_preserves_existing_budget_and_never_evicts_spent_tokens() {
    let mut probes = Probes::default();
    let now = Instant::now();
    for index in 1..=MAX_PENDING {
        probes
            .admit_rate(observation(index), RateRole::Remote, now)
            .unwrap();
    }
    assert_eq!(probes.rates.len(), MAX_PENDING);
    let excess = observation(MAX_PENDING + 1);
    assert_eq!(
        probes.admit_rate(excess, RateRole::Remote, now),
        Err(Error::Capacity)
    );
    probes
        .admit_rate(observation(1), RateRole::Remote, now)
        .unwrap();
    probes.maintain_rates(now + Duration::from_millis(999));
    assert_eq!(probes.rates.len(), MAX_PENDING);
    assert_eq!(
        probes.admit_rate(excess, RateRole::Remote, now + Duration::from_millis(999)),
        Err(Error::Capacity)
    );
    probes.maintain_rates(now + INTERVAL);
    assert_eq!(probes.rates.len(), 1);
    assert_eq!(probes.rates[&observation(1)].full_at, now + INTERVAL * 2);
    probes
        .admit_rate(excess, RateRole::Source, now + INTERVAL)
        .unwrap();
    assert_eq!(probes.rates.len(), 2);
}

//! Representative benchmark for the busiest pure API boundary.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use serde_json::{Value, json};
use tbc_insurance_mcp::api::normalize_coverage_benefits;

fn coverage_payload() -> Value {
    let risks = (0..32)
        .map(|index| {
            json!({
                "tagid": index,
                "name": "Outpatient care",
                "amount": "1000.00",
                "norate": "20%",
                "usedLimit": "125.50"
            })
        })
        .collect::<Vec<_>>();
    json!({"risks": risks})
}

fn benchmark_normalization(criterion: &mut Criterion) {
    let payload = coverage_payload();
    criterion.bench_function("normalize_32_coverage_benefits", |bencher| {
        bencher.iter_batched(
            || payload.clone(),
            |input| black_box(normalize_coverage_benefits(input).expect("valid fixture")),
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, benchmark_normalization);
criterion_main!(benches);

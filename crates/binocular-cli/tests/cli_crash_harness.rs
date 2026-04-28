// Default iterations are CI-safe.
// Set BINO_CRASH_ITERS to increase local stress coverage.

use binocular_core::buffer::MemoryBuffer;
use binocular_core::interpret::interpret_schema;
use binocular_schema::parser::parse_schema_str;
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use std::panic::{catch_unwind, AssertUnwindSafe};

const DEFAULT_ITERS: usize = 200;
const SCHEMA_MAX_BYTES: usize = 512;
const FILE_MAX_BYTES: usize = 2048;
const RNG_SEED: u64 = 0xB10C_0C1A_1234_5678;

fn crash_iters() -> usize {
    std::env::var("BINO_CRASH_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_ITERS)
}

#[test]
fn schema_and_interpretation_pipeline_is_panic_safe_for_random_inputs() {
    let mut rng = StdRng::seed_from_u64(RNG_SEED);

    for iteration in 0..crash_iters() {
        let schema_len = rng.gen_range(0..=SCHEMA_MAX_BYTES);
        let mut schema_bytes = vec![0_u8; schema_len];
        rng.fill_bytes(&mut schema_bytes);

        let file_len = rng.gen_range(0..=FILE_MAX_BYTES);
        let mut file_bytes = vec![0_u8; file_len];
        rng.fill_bytes(&mut file_bytes);

        let caught = catch_unwind(AssertUnwindSafe(|| {
            let schema_text = String::from_utf8_lossy(&schema_bytes);
            if let Ok(schema) = parse_schema_str(&schema_text) {
                let buffer = MemoryBuffer::from_vec(file_bytes);
                let _ = interpret_schema(&buffer, &schema);
            }
        }));

        assert!(caught.is_ok(), "pipeline panicked on iteration {iteration}");
    }
}

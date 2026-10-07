//! Fixed-iteration primitive benchmark. Explicit example, never an ordinary test.
#![forbid(unsafe_code)]
mod aead;
mod mac;

use aead::{Cipher, nonce};
use clap::Parser;
use serde::Serialize;
use std::{hint::black_box, io::Write, time::Instant};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Parser, Serialize)]
struct Arguments {
    #[arg(long, default_value_t = 100000, value_parser = clap::value_parser!(u32).range(100..=10000000))]
    iterations: u32,
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=100))]
    repeats: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sample {
    operation: String,
    bytes: usize,
    repeat: u32,
    iterations: u32,
    cpu_ns: u64,
    wall_ns: u128,
}

fn cpu() -> u64 {
    let value =
        nix::time::clock_gettime(nix::time::ClockId::CLOCK_THREAD_CPUTIME_ID).expect("CPU clock");
    u64::try_from(value.tv_sec()).expect("seconds") * 1_000_000_000
        + u64::try_from(value.tv_nsec()).expect("nanos")
}

struct Run {
    args: Arguments,
    samples: Vec<Sample>,
    repeat: u32,
    bytes: usize,
}

impl Run {
    fn time<T>(&mut self, name: &str, mut action: impl FnMut(u64) -> T) {
        for i in 0..1000 {
            black_box(action(i));
        }
        let start_cpu = cpu();
        let start = Instant::now();
        for i in 0..u64::from(self.args.iterations) {
            black_box(action(black_box(i + 1000)));
        }
        self.samples.push(Sample {
            operation: name.into(),
            bytes: self.bytes,
            repeat: self.repeat,
            iterations: self.args.iterations,
            cpu_ns: cpu() - start_cpu,
            wall_ns: start.elapsed().as_nanos(),
        });
    }

    fn aead<C: Cipher>(&mut self, name: &str) {
        let plaintext = vec![0xa5; self.bytes];
        let mut buffer = plaintext.clone();
        self.time(&format!("{name}.seal.inplace.new"), |i| {
            let tag = C::new().seal(nonce(i), &[], black_box(&mut buffer));
            black_box(&buffer);
            tag
        });
        let mut ciphertext = plaintext.clone();
        let tag = C::new().seal(nonce(0), &[], &mut ciphertext);
        self.time(&format!("{name}.open.inplace.new.copy"), |_| {
            buffer.copy_from_slice(black_box(&ciphertext));
            C::new().open(nonce(0), &[], black_box(&mut buffer), black_box(tag));
            black_box(&buffer);
        });
        self.time(&format!("{name}.seal.alloc.new"), |i| {
            let mut buffer = plaintext.clone();
            let tag = C::new().seal(nonce(i), &[], black_box(&mut buffer));
            black_box((buffer, tag))
        });
        self.time(&format!("{name}.open.alloc.new"), |_| {
            let mut buffer = ciphertext.clone();
            C::new().open(nonce(0), &[], black_box(&mut buffer), black_box(tag));
            black_box(buffer)
        });
        self.time(&format!("{name}.aad_tag.new"), |i| {
            C::new().seal(nonce(i), black_box(&plaintext), &mut [])
        });
        let mut frame = mac::wire_frame(self.bytes);
        self.time(&format!("{name}.aad_tag.v5.new"), |i| {
            frame[42..50].copy_from_slice(&i.to_be_bytes());
            C::new().seal(nonce(i), black_box(&frame), &mut [])
        });
    }
}

fn run() -> Result<(), Error> {
    meshway_logging::Options {
        filter: "info".into(),
        format: meshway_logging::Format::Json,
    }
    .init()?;
    let mut run = Run {
        args: Arguments::parse(),
        samples: Vec::new(),
        repeat: 0,
        bytes: 0,
    };
    for repeat in 0..run.args.repeats {
        run.repeat = repeat;
        for bytes in [52, 1280] {
            run.bytes = bytes;
            aead::validate(bytes);
            let input = vec![0x5a; bytes];
            let mut output = input.clone();
            run.time("copy", |_| {
                output.copy_from_slice(black_box(&input));
                black_box(&output);
            });
            // Reverse provider ordering on alternate repeats to expose drift.
            if repeat % 2 == 0 {
                run.aead::<aead::Old>("chacha0.10");
                run.aead::<aead::New>("chacha0.11");
                run.aead::<aead::Ring>("ring");
            } else {
                run.aead::<aead::Ring>("ring");
                run.aead::<aead::New>("chacha0.11");
                run.aead::<aead::Old>("chacha0.10");
            }
            mac::measure(&mut run, &input);
            mac::v5(&mut run);
        }
    }
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &run.samples)?;
    writeln!(output)?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Crypto measurement failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

//! Claim and release of a producer slot in a peer directory, by one thread
//! and by four threads at once.

use std::sync::Arc;
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion};
use subetha_cxc::peer_directory::PeerDirectory;

fn claim_release(c: &mut Criterion) {
    let dir = PeerDirectory::create_anon().expect("a directory");
    c.bench_function("peer_directory.claim_release/1", |b| {
        b.iter(|| {
            let slot = dir.claim_producer_slot().expect("a free slot");
            dir.release_producer_slot(slot);
        })
    });
    c.bench_function("peer_directory.claim_release/4", |b| {
        b.iter_custom(|iters| {
            let dir = Arc::new(PeerDirectory::create_anon().expect("a directory"));
            let per = iters.div_ceil(4);
            let start = Instant::now();
            let threads: Vec<_> = (0..4)
                .map(|_| {
                    let dir = Arc::clone(&dir);
                    std::thread::spawn(move || {
                        for _ in 0..per {
                            let slot = dir.claim_producer_slot().expect("a free slot");
                            dir.release_producer_slot(slot);
                        }
                    })
                })
                .collect();
            for t in threads {
                t.join().expect("a claiming thread");
            }
            start.elapsed()
        })
    });
}

criterion_group!(benches, claim_release);
criterion_main!(benches);

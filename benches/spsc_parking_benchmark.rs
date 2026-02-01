use std::{
    hint::black_box,
    num::NonZeroUsize,
    ptr,
    thread::spawn,
    time::{Duration, SystemTime},
};

use criterion::{
    BenchmarkGroup, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};
use gil::spsc::parking::channel;

#[derive(Clone, Copy)]
#[repr(align(8))]
struct Payload1024 {
    data: [u8; 1024],
}

impl Payload1024 {
    fn new(val: u8) -> Self {
        Self { data: [val; 1024] }
    }
}

fn make_group<'a>(c: &'a mut Criterion, name: &str) -> BenchmarkGroup<'a, WallTime> {
    let mut group = c.benchmark_group(name);
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(50);
    group.warm_up_time(Duration::from_secs(1));

    group
}

fn benchmark(c: &mut Criterion) {
    const SIZES: [NonZeroUsize; 3] = [
        NonZeroUsize::new(512).unwrap(),
        NonZeroUsize::new(4096).unwrap(),
        NonZeroUsize::new(65_536).unwrap(),
    ];

    // ==================== ROUNDTRIP LATENCY ====================
    let mut group = make_group(c, "parking_roundtrip_latency");

    for size in SIZES {
        group.bench_function(format!("size_{size}/payload_1"), |b| {
            b.iter_custom(move |iter| {
                let (mut tx1, mut rx1) = channel::<u8>(size);
                let (mut tx2, mut rx2) = channel::<u8>(size);

                let iter = iter as usize;

                spawn(move || {
                    for i in 0..iter {
                        let x = rx1.recv();
                        black_box(x);
                        tx2.send(black_box(i as u8));
                    }
                });

                let start = SystemTime::now();

                for i in 0..iter {
                    tx1.send(black_box(i as u8));
                    let x = rx2.recv();
                    black_box(x);
                }

                start.elapsed().unwrap()
            });
        });

        group.bench_function(format!("size_{size}/payload_8"), |b| {
            b.iter_custom(move |iter| {
                let (mut tx1, mut rx1) = channel::<usize>(size);
                let (mut tx2, mut rx2) = channel::<usize>(size);

                let iter = iter as usize;

                spawn(move || {
                    for i in 0..iter {
                        let x = rx1.recv();
                        black_box(x);
                        tx2.send(black_box(i));
                    }
                });

                let start = SystemTime::now();

                for i in 0..iter {
                    tx1.send(black_box(i));
                    let x = rx2.recv();
                    black_box(x);
                }

                start.elapsed().unwrap()
            });
        });

        group.bench_function(format!("size_{size}/payload_1024"), |b| {
            b.iter_custom(move |iter| {
                let (mut tx1, mut rx1) = channel::<Payload1024>(size);
                let (mut tx2, mut rx2) = channel::<Payload1024>(size);

                let iter = iter as usize;

                spawn(move || {
                    for i in 0..iter {
                        let x = rx1.recv();
                        black_box(x);
                        tx2.send(black_box(Payload1024::new(i as u8)));
                    }
                });

                let start = SystemTime::now();

                for i in 0..iter {
                    tx1.send(black_box(Payload1024::new(i as u8)));
                    let x = rx2.recv();
                    black_box(x);
                }

                start.elapsed().unwrap()
            });
        });
    }
    drop(group);

    // ==================== PUSH LATENCY ====================
    let mut group = make_group(c, "parking_push_latency");

    for size in SIZES {
        group.bench_function(format!("size_{size}/payload_1"), |b| {
            b.iter_custom(|iter| {
                let (mut tx, mut rx) = channel::<u8>(size);
                spawn(move || {
                    for _ in 0..iter {
                        black_box(rx.recv());
                    }
                });
                let start = SystemTime::now();

                for _ in 0..iter {
                    tx.send(black_box(0u8));
                }

                start.elapsed().unwrap()
            });
        });

        group.bench_function(format!("size_{size}/payload_8"), |b| {
            b.iter_custom(|iter| {
                let (mut tx, mut rx) = channel::<usize>(size);
                spawn(move || {
                    for _ in 0..iter {
                        black_box(rx.recv());
                    }
                });
                let start = SystemTime::now();

                for _ in 0..iter {
                    tx.send(black_box(0usize));
                }

                start.elapsed().unwrap()
            });
        });

        group.bench_function(format!("size_{size}/payload_1024"), |b| {
            b.iter_custom(|iter| {
                let (mut tx, mut rx) = channel::<Payload1024>(size);
                spawn(move || {
                    for _ in 0..iter {
                        black_box(rx.recv());
                    }
                });
                let start = SystemTime::now();

                for _ in 0..iter {
                    tx.send(black_box(Payload1024::new(0)));
                }

                start.elapsed().unwrap()
            });
        });
    }
    drop(group);

    // ==================== THROUGHPUT ====================
    const ELEMENTS: usize = 1_000_000;

    // Throughput for u8 (1 byte)
    {
        let mut group = make_group(c, "parking_throughput/payload_1");
        group.throughput(Throughput::ElementsAndBytes {
            elements: ELEMENTS as u64,
            bytes: (ELEMENTS * size_of::<u8>()) as u64,
        });

        for size in SIZES {
            group.bench_function(format!("size_{size}/direct"), |b| {
                b.iter(|| {
                    let (mut tx, mut rx) = channel::<u8>(size);

                    spawn(move || {
                        for i in 0..ELEMENTS {
                            let x = black_box(rx.recv());
                            assert_eq!((i & 0xFF) as u8, x);
                        }
                    });

                    for i in 0..ELEMENTS {
                        tx.send(black_box((i & 0xFF) as u8));
                    }
                });
            });

            group.bench_function(format!("size_{size}/batched"), |b| {
                b.iter(|| {
                    let (mut tx, mut rx) = channel::<u8>(size);

                    spawn(move || {
                        let mut received = 0;
                        while received < ELEMENTS {
                            let buf = rx.read_buffer();
                            let len = buf.len();
                            if len == 0 {
                                core::hint::spin_loop();
                                continue;
                            }

                            black_box(buf[0]);

                            unsafe { rx.advance(len) };
                            received += len;
                        }
                    });

                    let mut sent = 0;
                    let src_data = vec![1u8; size.get()];

                    while sent < ELEMENTS {
                        let buf = tx.write_buffer();
                        let len = buf.len().min(ELEMENTS - sent).min(src_data.len());
                        if len == 0 {
                            core::hint::spin_loop();
                            continue;
                        }

                        unsafe {
                            ptr::copy_nonoverlapping(
                                src_data.as_ptr().cast(),
                                buf.as_mut_ptr(),
                                len,
                            );
                        }

                        unsafe { tx.commit(len) };
                        sent += len;
                    }
                });
            });
        }
        drop(group);
    }

    // Throughput for usize (8 bytes)
    {
        let mut group = make_group(c, "parking_throughput/payload_8");
        group.throughput(Throughput::ElementsAndBytes {
            elements: ELEMENTS as u64,
            bytes: (ELEMENTS * size_of::<usize>()) as u64,
        });

        for size in SIZES {
            group.bench_function(format!("size_{size}/direct"), |b| {
                b.iter(|| {
                    let (mut tx, mut rx) = channel::<usize>(size);

                    spawn(move || {
                        for i in 0..ELEMENTS {
                            let x = black_box(rx.recv());
                            assert_eq!(i, x);
                        }
                    });

                    for i in 0..ELEMENTS {
                        tx.send(black_box(i));
                    }
                });
            });

            group.bench_function(format!("size_{size}/batched"), |b| {
                b.iter(|| {
                    let (mut tx, mut rx) = channel::<usize>(size);

                    spawn(move || {
                        let mut received = 0;
                        while received < ELEMENTS {
                            let buf = rx.read_buffer();
                            let len = buf.len();
                            if len == 0 {
                                core::hint::spin_loop();
                                continue;
                            }

                            black_box(buf[0]);

                            unsafe { rx.advance(len) };
                            received += len;
                        }
                    });

                    let mut sent = 0;
                    let src_data = vec![1usize; size.get()];

                    while sent < ELEMENTS {
                        let buf = tx.write_buffer();
                        let len = buf.len().min(ELEMENTS - sent).min(src_data.len());
                        if len == 0 {
                            core::hint::spin_loop();
                            continue;
                        }

                        unsafe {
                            ptr::copy_nonoverlapping(
                                src_data.as_ptr().cast(),
                                buf.as_mut_ptr(),
                                len,
                            );
                        }

                        unsafe { tx.commit(len) };
                        sent += len;
                    }
                });
            });
        }
        drop(group);
    }

    // Throughput for 1024-byte payload
    {
        let mut group = make_group(c, "parking_throughput/payload_1024");
        const LARGE_ELEMENTS: usize = 100_000;
        group.throughput(Throughput::ElementsAndBytes {
            elements: LARGE_ELEMENTS as u64,
            bytes: (LARGE_ELEMENTS * size_of::<Payload1024>()) as u64,
        });

        for size in SIZES {
            group.bench_function(format!("size_{size}/direct"), |b| {
                b.iter(|| {
                    let (mut tx, mut rx) = channel::<Payload1024>(size);

                    spawn(move || {
                        for i in 0..LARGE_ELEMENTS {
                            let x = black_box(rx.recv());
                            assert_eq!((i & 0xFF) as u8, x.data[0]);
                        }
                    });

                    for i in 0..LARGE_ELEMENTS {
                        tx.send(black_box(Payload1024::new((i & 0xFF) as u8)));
                    }
                });
            });

            group.bench_function(format!("size_{size}/batched"), |b| {
                b.iter(|| {
                    let (mut tx, mut rx) = channel::<Payload1024>(size);

                    spawn(move || {
                        let mut received = 0;
                        while received < LARGE_ELEMENTS {
                            let buf = rx.read_buffer();
                            let len = buf.len();
                            if len == 0 {
                                core::hint::spin_loop();
                                continue;
                            }

                            black_box(buf[0]);

                            unsafe { rx.advance(len) };
                            received += len;
                        }
                    });

                    let mut sent = 0;
                    let src_data: Vec<Payload1024> = (0..size.get())
                        .map(|i| Payload1024::new((i & 0xFF) as u8))
                        .collect();

                    while sent < LARGE_ELEMENTS {
                        let buf = tx.write_buffer();
                        let len = buf.len().min(LARGE_ELEMENTS - sent).min(src_data.len());
                        if len == 0 {
                            core::hint::spin_loop();
                            continue;
                        }

                        unsafe {
                            ptr::copy_nonoverlapping(
                                src_data.as_ptr().cast(),
                                buf.as_mut_ptr(),
                                len,
                            );
                        }

                        unsafe { tx.commit(len) };
                        sent += len;
                    }
                });
            });
        }
    }
}

criterion_group! {benches, benchmark}
criterion_main! {benches}

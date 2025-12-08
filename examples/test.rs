use gil::channel;
use std::{hint::black_box, num::NonZeroUsize, thread::spawn, time::SystemTime};

fn main() {
    let (mut tx, mut rx) = channel(NonZeroUsize::new(4096).unwrap());

    let start = SystemTime::now();

    const COUNTS: usize = 100_000_000;

    spawn(move || {
        for _ in 0..COUNTS {
            let x = rx.recv();
            black_box(x);
        }
    });

    for i in 0..COUNTS {
        tx.send(black_box(i));
    }

    let time = start.elapsed().unwrap();
    println!("{time:?}");
}

use gil::mpsc::channel;
use std::{hint::black_box, num::NonZeroUsize, thread::spawn, time::SystemTime};

fn main() {
    let (tx, mut rx) = channel(NonZeroUsize::new(4096).unwrap());

    const SENDERS: usize = 8;
    const MESSAGES: usize = 1_000_000;

    let start = SystemTime::now();

    for _ in 0..SENDERS {
        let mut tx = tx.clone();
        spawn(move || {
            for i in 0..MESSAGES {
                tx.send(black_box(i));
            }
        });
    }

    // Drop the original sender since we've cloned it for all threads
    drop(tx);

    for _ in 0..(SENDERS * MESSAGES) {
        let x = rx.recv();
        black_box(x);
    }

    let time = start.elapsed().unwrap();
    println!("{time:?}");
}

this is a collection of multithreaded queues, most of them lock free. code organization:

- src/queue.rs: a generic ring buffer
- src/backoff.rs: common backoff logic for queues which need it
- src/cell.rs: cache-aligned cell with epoch counter, used by mpmc/mpsc/spmc
- src/padded.rs: cache-line padding wrapper to prevent false sharing
- src/spsc: lock free, based on folly's ProducerConsumerQueue, supports zero-copy batch ops
- src/mpsc: based on vyukov's bounded mpmc, optimized for single consumer
- src/spmc: based on vyukov's bounded mpmc, optimized for single producer
- src/mpmc: based on vyukov's bounded mpmc
- src/{mpsc,mpmc}/sharded: multiple internal spsc queues to reduce contention, bounded shard count

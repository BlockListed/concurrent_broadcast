use std::sync::Arc;
use std::time::Instant;

use futures_util::stream::FuturesOrdered;
use futures_util::{FutureExt, StreamExt};
use tokio::sync::Barrier;

use crate::{channel, Receiver, Sender};

const RECEIVER_COUNT: usize = 2_000_000;
const MESSAGE_COUNT: i32 = 50;

#[tokio::test(flavor = "multi_thread")]
async fn basic_test() {
    let (tx, rx) = channel();

    tx.send(-1);

    let first_recv_barrier = Arc::new(Barrier::new(RECEIVER_COUNT + 1));
    let finished_barrier = Arc::new(Barrier::new(RECEIVER_COUNT + 1));

    let recv: FuturesOrdered<_> = (0..RECEIVER_COUNT)
        .map(|_| {
            let rx_clone = rx.clone();
            let barrier_clone = first_recv_barrier.clone();
            let done_barrier = finished_barrier.clone();
            tokio::spawn(receiver_task(barrier_clone, done_barrier, rx_clone)).map(|r| r.unwrap())
        })
        .collect();

    // make sure all tasks are syncronized
    first_recv_barrier.wait().await;

    let start = Instant::now();

    let send = tokio::spawn(sender_task(tx));

    // waiting for all tasks to be done seems to take longer
    finished_barrier.wait().await;
    let elapsed = start.elapsed();

    recv.collect::<()>().await;
    send.await.unwrap();

    println!(
        "Took {}ms to send {} messages.",
        elapsed.as_millis(),
        RECEIVER_COUNT * MESSAGE_COUNT as usize
    )
}

async fn sender_task(tx: Sender<i32>) {
    let start = Instant::now();
    for i in 0..MESSAGE_COUNT {
        tx.send(i);
    }
    let elapsed = start.elapsed();
    println!("Took {}ms to send {MESSAGE_COUNT} messages to {RECEIVER_COUNT} receivers.", elapsed.as_millis());
}

async fn receiver_task(barrier: Arc<Barrier>, done_barrier: Arc<Barrier>, mut rx: Receiver<i32>) {
    assert_eq!(rx.recv().await, -1);

    barrier.wait().await;

    drop(barrier);

    for _ in 0..MESSAGE_COUNT {
        rx.recv().await;
    }

    done_barrier.wait().await;
}

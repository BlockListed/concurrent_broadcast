//! # concurrent_broadcast
//! A high-performance guaranteed delivery broadcast channel.
//! <div class="warning">
//! Due to its delivery guarantees, this channel is vulnerable to
//! the "slow-receiver" problem. If a receiver doesn't `recv` messages
//! fast enough, old messages will accumulate, since they won't be dropped
//! until they are seen by the slow sender.
//! </div>
//!
//! # Example
//!
//! ```rust
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! use concurrent_broadcast::channel;
//! let (tx, rx) = channel::<&'static str>();
//! 
//! for _ in 0..5 {
//!     let local_rx = rx.clone();
//!     tokio::spawn(async move {
//!         println!("Received: {}", local_rx.recv().await.unwrap());
//!     });
//! }
//! 
//! tx.send("Hello, World!").unwrap();
//! # })
//! ```
//! 
//! # Expected output
//! ```text
//! Received: Hello, World!
//! Received: Hello, World!
//! Received: Hello, World!
//! Received: Hello, World!
//! Received: Hello, World!
//! ```


use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, OnceLock,
};

use parking_lot::RwLock;
use tokio::sync::Notify;

/// Create a new channel.
/// Comprised of a [`Sender`] and a [`Receiver`].
pub fn channel<T: Clone>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Internal {
        head: OnceLock::new(),
        notify: Notify::new(),
        tx_cnt: AtomicUsize::new(1),
        rx_cnt: AtomicUsize::new(1),
    });

    (
        Sender {
            shared: shared.clone(),
        },
        Receiver {
            shared,
            current: None,
        },
    )
}

pub struct Sender<T> {
    shared: Arc<Internal<T>>,
}

impl<T> Sender<T> {
    /// Attempts to send a value on the channel.
    /// # Errors
    /// `Err(_)`` indicates that there are no active receivers, (the channel is closed)
    /// `Ok(())`` however does not guarantee, that the message will
    /// be seen by ANY receiver, since they may be dropped while
    /// the message is being sent.

    pub fn send(&self, t: T) -> Result<(), T> {
        let t = self.check_closed(t)?;

        let node = Arc::new(Node {
            next: OnceLock::new(),
            data: t,
        });

        let mut head = match self.shared.head.get() {
            Some(t) => t.write(),
            None => {
                let head = RwLock::new(node.clone());
                match self.shared.head.set(head) {
                    Ok(()) => {
                        self.shared.notify.notify_waiters();
                        return Ok(());
                    }
                    Err(_) => self.shared.head.get().unwrap().write(),
                }
            }
        };

        Self::do_send(&*head, node.clone());

        *head = node;

        drop(head);

        self.shared.notify.notify_waiters();
        Ok(())
    }

    fn do_send(mut head: &Arc<Node<T>>, node: Arc<Node<T>>) {
        let mut node = Some(node);

        loop {
            match head.next.set(node.take().unwrap()) {
                Ok(()) => break,
                Err(t) => {
                    head = head.next.get().unwrap();
                    node = Some(t);
                }
            }
        }
    }

    // returns value, so that we can ? on this in the main send function
    fn check_closed(&self, t: T) -> Result<T, T> {
        if self.shared.rx_cnt.load(Ordering::Relaxed) == 0 {
            return Err(t);
        }

        Ok(t)
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        let shared = self.shared.clone();
        self.shared.tx_cnt.fetch_add(1, Ordering::Relaxed);

        Self { shared }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if self.shared.tx_cnt.fetch_sub(1, Ordering::Relaxed) == 0 {
            // notify all waiting tasks, that the channel is closed
            self.shared.notify.notify_waiters();
        }
    }
}

pub struct Receiver<T> {
    shared: Arc<Internal<T>>,
    current: Option<Arc<Node<T>>>,
}

impl<T: Clone> Receiver<T> {
    /// Attempts to receive a message from the channel.
    /// If this is the first time this receiver has been used,
    /// it will choose the newest message as its starting point.
    /// Any messages sent before the newest message WILL NOT BE VISIBLE.
    /// Cloned versions of a receiver will start seeing messages that
    /// were sent after the newest message the receiver received
    /// before being cloned.
    /// # Errors
    /// When there are no active `Senders` AND there are no more messages. (The channel is closed)
    pub async fn recv(&mut self) -> Result<T, ()> {
        let current = match self.current {
            Some(ref n) => n.clone(),
            None => {
                let head = self.get_head().await?.read().clone();

                let data = head.data.clone();

                self.current = Some(head);

                return Ok(data);
            }
        };

        let next = self.get_next(current).await?;

        let data = next.data.clone();

        self.current = Some(next);

        Ok(data)
    }

    async fn get_head(&self) -> Result<&RwLock<Arc<Node<T>>>, ()> {
        loop {
            match self.shared.head.get() {
                Some(head) => break Ok(head),
                None => {
                    self.check_closed()?;
                    self.shared.notify.notified().await;
                }
            }
        }
    }

    async fn get_next(&self, node: Arc<Node<T>>) -> Result<Arc<Node<T>>, ()> {
        loop {
            match node.next.get() {
                Some(next) => break Ok(next.clone()),
                None => {
                    self.check_closed()?;
                    self.shared.notify.notified().await;
                }
            }
        }
    }

    fn check_closed(&self) -> Result<(), ()> {
        if self.shared.tx_cnt.load(Ordering::Relaxed) == 0 {
            return Err(());
        }

        Ok(())
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        let shared = self.shared.clone();
        let current = self.current.clone();

        self.shared.rx_cnt.fetch_add(1, Ordering::Relaxed);

        Self { shared, current }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shared.rx_cnt.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Internal<T> {
    // refers to the newest element
    head: OnceLock<RwLock<Arc<Node<T>>>>,
    notify: Notify,
    tx_cnt: AtomicUsize,
    rx_cnt: AtomicUsize,
}

struct Node<T> {
    next: OnceLock<Arc<Node<T>>>,
    data: T,
}

#[cfg(test)]
mod test;

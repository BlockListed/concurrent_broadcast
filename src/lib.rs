use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;
use tokio::sync::Notify;

pub fn channel<T: Clone>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Internal {
        head: OnceLock::new(),
        notify: Notify::new(),
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

#[derive(Clone)]
pub struct Sender<T> {
    shared: Arc<Internal<T>>,
}

impl<T: Clone> Sender<T> {
    pub fn send(&self, t: T) {
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
                        return;
                    }
                    Err(_) => self.shared.head.get().unwrap().write(),
                }
            }
        };

        Self::do_send(&*head, node.clone());

        *head = node;

        drop(head);

        self.shared.notify.notify_waiters();
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
}

#[derive(Clone)]
pub struct Receiver<T> {
    shared: Arc<Internal<T>>,
    current: Option<Arc<Node<T>>>,
}

impl<T: Clone> Receiver<T> {
    pub async fn recv(&mut self) -> T {
        let current = match self.current {
            Some(ref n) => n.clone(),
            None => {
                let head = self.get_head().await.read().clone();

                let data = head.data.clone();

                self.current = Some(head);

                return data;
            }
        };

        let next = self.get_next(current).await;

        let data = next.data.clone();

        self.current = Some(next);

        data
    }

    async fn get_head(&self) -> &RwLock<Arc<Node<T>>> {
        loop {
            match self.shared.head.get() {
                Some(head) => break head,
                None => {
                    self.shared.notify.notified().await;
                }
            }
        }
    }

    async fn get_next(&self, node: Arc<Node<T>>) -> Arc<Node<T>> {
        loop {
            match node.next.get() {
                Some(next) => break next.clone(),
                None => {
                    self.shared.notify.notified().await;
                }
            }
        }
    }
}

pub struct Internal<T> {
    // refers to the newest element
    head: OnceLock<RwLock<Arc<Node<T>>>>,
    notify: Notify,
}

#[derive(Debug)]
pub struct Node<T> {
    next: OnceLock<Arc<Node<T>>>,
    data: T,
}

#[cfg(test)]
mod test;

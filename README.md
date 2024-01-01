# concurrent_broadcast
A blazingly fast 🚀 (tm) async broadcast channel,
with guaranteed delivery.

## Working theory
It operates on a concurrent Linked list with `Arc`s as the
pointers between elements. The first time a reader calls `recv`
it saves the newest item as its "current" element, further
calls simply walk the linked list and wait for new elements
when the end is reached. The only reference stored by the channel
itself is the newest item. 

## WARNING: "slow-receiver" problem
As you can imagine, if a reader continues
storing a very old item, any newer item then can no longer be dropped,
until the reader advances its position, which results in this implementation
being vulnerable to the "slow-receiver" problem. `tokio::sync::broadcast`
doesn't have this weakness although the trade-off is, that it doesn't
have guaranteed delivery.

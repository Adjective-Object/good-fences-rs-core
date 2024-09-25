# deadlock_ref
This crate provides a wrapper for debugging deadlocks in dashmap.

It works based on timeouts, logging when a thread overruns a timeout while holding a reference or while waiting to grab a reference. This is horrendously inefficient, spawning a new thread each time it is invoked. DO NOT USE IN PRODUCTION.
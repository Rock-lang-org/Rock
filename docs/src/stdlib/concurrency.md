# Threads and Synchronization

The standard library provides owned POSIX threads, atomic reference counting, a spin-yield mutex, and one 64-bit atomic type. These APIs make ownership requirements visible: a spawned closure must own `Send` data, a join handle provides the option to join, and a mutex guard unlocks when it is dropped.

## Spawning and joining

`spawn` is an explicitly imported function. It accepts an owned `FnOnce` task whose captured environment and result implement `Send`, and returns `Result (JoinHandle T), ThreadError`. `join!` consumes the handle and returns the task result or a concrete `ThreadError`.

```rock
> stdlib::thread::spawn

main = !->
    match spawn (-> 42)
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
```

The output is `42`. A successful `spawn` transfers the closure to the new thread; `join!` transfers its result back. Dropping an unjoined handle detaches the thread without blocking, and the worker cleans up its result when it finishes. A failed create or join is data in `ThreadError`, not an exception.

## Detached tasks

Use the explicitly imported `spawn_detached` when no join handle is needed. It accepts an owned `Send` task implementing `FnOnce (), ()` and returns `Result (), ThreadError`: success reports that the thread started, not that the task finished.

```rock
> stdlib::thread::spawn_detached

main = ->
    match spawn_detached (->
        "background task".println!
        return)
        Result::Ok _ => 0
        Result::Err _ =>
            "spawn failed".println!
            1
```

The explicit `return` makes the task return unit (`()`), rather than the result of `println!`. There is deliberately no expected worker output: returning from `main` does not wait for detached tasks, so the process can exit before the message is printed. Sleeping is not a completion guarantee; use `spawn` and `join!` when the work must finish before exit. Handle any recoverable task errors inside the task, since `spawn_detached` only reports startup failure.

## Owned captures

A spawned closure cannot borrow a stack value that might end before the thread. Move owned data into the closure by using a consuming method or an owned container.

```rock
> stdlib::thread::spawn

struct Owned
    < value: I64

impl Owned
    ~@take: I64
    ~@take = -> self.value

main = !->
    owned = Owned
        value: 73
    match spawn (-> owned.take!)
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
```

The output is `73`. The consuming `owned.take!` call makes the closure capture `owned` by value, and the worker consumes it when the closure runs. A closure that instead borrowed the local `owned` would not satisfy the required thread-safety bounds.

## Shared mutable state

Use `Arc (Mutex T)` when several owned workers must update one value. `Arc::clone` shares ownership; `Mutex::lock!` gives one worker a `MutexGuard T`; `get_mut!` changes the protected value; and dropping the guard releases the lock.

```rock
> stdlib::arc::Arc
> stdlib::sync::Mutex
> stdlib::thread::spawn

struct Counter
    < value: I64

impl Counter
    ^@increment: () -> ()
    ^@increment = ->
        self.value = self.value + 1
        return

struct Worker
    < counter: Arc (Mutex Counter)
    < iterations: I64

impl Worker
    ~@run: I64
    ~@run = ->
        mut index: I64 = 0
        while index < self.iterations
            mut guard = self.counter.lock!
            guard.get_mut!.increment!
            index = index + 1
        index

main = !->
    counter: Arc (Mutex Counter) = Arc::new (Mutex::new (Counter
        value: 0))
    first = Worker
        counter: counter.clone!
        iterations: 1000
    second = Worker
        counter: counter.clone!
        iterations: 1000
    first_handle = spawn (-> first.run!)
    second_handle = spawn (-> second.run!)
    match first_handle
        Result::Ok handle =>
            match handle.join!
                Result::Ok _ => 0
                Result::Err _ => 1
        Result::Err _ => 1
    match second_handle
        Result::Ok handle =>
            match handle.join!
                Result::Ok _ => 0
                Result::Err _ => 1
        Result::Err _ => 1
    guard = counter.lock!
    guard.get!.value.println!
```

Both workers own a cloned `Arc`, but only one guard can update the `Counter` at a time. The final output is `2000` when both threads start and join successfully. Keep lock scopes short, never call unknown code while holding a guard, and establish one order before acquiring several mutexes. `Arc` supplies ownership; it does not supply mutation or synchronization by itself.

## Guard lifetime and `try_lock!`

`lock!` waits by yielding while the lock word is held. `try_lock!` returns immediately with `Option (MutexGuard T)`. A guard's lifetime controls the lock lifetime, so the second attempt below returns `None` until the first guard is dropped.

```rock
> stdlib::sync::Mutex

lock_once: &Mutex I64 -> ()
lock_once = mutex ->
    guard = mutex.lock!
    match mutex.try_lock!
        Option::Some _ => "unexpected lock".println!
        Option::None => "locked".println!
    return

main = !->
    mutex: Mutex I64 = Mutex::new 9
    lock_once (&mutex)
    match mutex.try_lock!
        Option::Some guard => guard.get!.println!
        Option::None => "still locked".println!
```

The output is `locked` and `9`. `guard` leaves scope before `lock_once` returns, so the later `try_lock!` succeeds. A guard is not `Send`; moving it into a spawned closure is rejected, which prevents a worker from unlocking a mutex through a guard created on another thread.

## Atomics

`AtomicU64` is an explicitly imported heap-backed atomic value. `exchange`, `fetch_add`, `fetch_sub`, and `store` return or update exact `U64` values.

```rock
> stdlib::atomic::AtomicU64

main = !->
    counter: AtomicU64 = AtomicU64::new 0 as U64
    exchanged: U64 = counter.exchange 3 as U64
    previous: U64 = counter.fetch_add 1 as U64
    counter.store 5 as U64
    next: U64 = counter.fetch_sub 2 as U64
    exchanged as I64 .println!
    previous as I64 .println!
    next as I64 .println!
```

The output is `0`, `3`, and `5`: `exchange` observes zero and writes three, `fetch_add` observes three, `store` writes five, and `fetch_sub` observes five before subtracting two. Atomics are appropriate for counters and simple protocols, not for a compound invariant that requires several reads and writes to change together.

## Scheduling helpers

The thread module also exports `current_id`, `yield_now`, and `sleep_ms`. They are explicit imports because they are not prelude vocabulary.

```rock
> stdlib::thread::current_id
> stdlib::thread::sleep_ms
> stdlib::thread::yield_now

main = !->
    thread_id: U64 = current_id!
    thread_id as I64 .println!
    yield_now!
    sleep_ms 1
```

The first line is a host-specific pthread identifier; the program then yields and sleeps for approximately one millisecond. Neither helper creates parallel work or guarantees a scheduling order.

## Current limits and common mistakes

- The implementation is pthread-oriented and assumes a Linux-style ABI.
- `Mutex` uses an atomic word plus `sched_yield`, not a platform blocking primitive.
- There are no channels, condition variables, thread pools, async functions, or executors.
- Dropping a `JoinHandle` detaches the thread; `spawn_detached` starts a unit task without returning a handle. Neither makes process exit wait for completion.
- Closure-environment cleanup still has incomplete cases; treat concurrency as a prototype runtime feature.
- Do not capture a reference, a `MutexGuard`, or an owned struct that contains a reference in a spawned closure.

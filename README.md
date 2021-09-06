# send_safe
> Safely convert Rust types to `Send`.

Suppose you have a variable `x` of type `T` which is not `Send`.
We want to use `x` from other threads.
But `T` is not `Send`, so we cannot move `x` across threads.

This crate provides `SendWrapperThread` which will create `x` in its own thread.
Then you can interact with `x` from other threads through the wrapper.

This is safe because `x` always stays on its own dedicated thread.
It's only the communication with it that needs to cross thread boundaries.
The thread exits and `x` is dropped when the wrapper is dropped.

## Example Usage
Suppose `x` is a raw pointer, which is not `Send`:

```rust
use send_safe::SendWrapperThread;

let make_x = || Box::into_raw(Box::new(41));
let mut wrapper = SendWrapperThread::new(make_x);

// Use `wrapper` to interact with `x` from inside a different thread.
std::thread::spawn(move || {
    let x_plus_1 = wrapper.execute(|x| {
        // The Box is just for demonstrating wrapping a raw pointer.
        // This doesn't have to be unsafe if you were using different types.
        let unboxed_x = unsafe { Box::from_raw(x) };
        (Some(x), *unboxed_x + 1)
    }).unwrap();
    assert_eq!(x_plus_1, 42);
}).join().unwrap();
```

## Acknowledgements
Thanks to [@pirocks](https://github.com/pirocks/) for advice and debugging help.

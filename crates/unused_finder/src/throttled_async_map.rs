use futures::future::{Future, FutureExt};
use tokio::task::{JoinError, JoinSet};

/// Run a function on the results of async operations, with a limit on the number of concurrent
/// async operations to be run.
///
pub async fn throttled_async_map<T, OutFuture, R>(
    // The maximum number of async operations to run concurrently.
    async_throttle_limit: usize,
    // The async operations to run concurrently.
    inputs: Vec<T>,
    // The function to run on the results of the async operations.
    result_processor: impl Copy + Sync + Send + 'static + Fn(T) -> OutFuture,
) -> Result<Vec<R>, JoinError>
where
    T: Send,
    OutFuture: Send + 'static + Future<Output = R> + FutureExt,
    R: Send + 'static,
{
    let limit = std::cmp::min(async_throttle_limit, inputs.len());
    if limit == 0 {
        return Ok(vec![]);
    }

    let mut join_set: JoinSet<(usize, R)> = JoinSet::new();
    // spawn the initial tasks
    for i in 0..limit {
        join_set.spawn(result_processor(inputs[i]).then(async move |v| (i, v)));
    }

    // wait for the next task to finish to spawn a new one, while
    // accumulating outputs in the output vector.
    let mut i = limit;
    let mut output: Vec<R> = Vec::with_capacity(inputs.len());
    unsafe {
        // UNSAFE: expand the vector to the full size of the inputs so we can
        // write into it later.
        output.set_len(inputs.len());
    }
    while let Some(joined) = join_set.join_next().await {
        let (result_i, result_value) = match joined {
            Err(e) => {
                // abort all outstanding futures
                join_set.abort_all();
                // return the error
                return Err(e);
            }
            Ok(v) => v,
        };
        // UNSAFE: We want to use `mem::replace` to avoid running the destructor
        // of the previous value, because it is uninitialized memory
        std::mem::replace(&mut output[result_i], result_value);

        i += 1;
        if i < inputs.len() {
            // spawn the next task, if there is a next task to spawn
            join_set.spawn(result_processor(inputs[i]).then(async move |v| (i, v)));
        }
    }

    Ok(output)
}

pub async fn throttled_async_wait<InFuture, R>(
    async_throttle_limit: usize,
    inputs: Vec<InFuture>,
) -> Result<Vec<R>, JoinError>
where
    R: Send + 'static,
    InFuture: Send + 'static + Future<Output = R> + FutureExt,
{
    throttled_async_map(async_throttle_limit, inputs, |x| x).await
}

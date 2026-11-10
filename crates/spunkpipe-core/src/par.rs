use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Runs `f` over `items` on up to `threads` threads, returning results in input order.
pub fn par_map<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = items.len();
    let threads = threads.clamp(1, n.max(1));
    if threads == 1 {
        return items.iter().map(&f).collect();
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let r = f(&items[i]);
                results.lock().unwrap()[i] = Some(r);
            });
        }
    });
    results.into_inner().unwrap().into_iter().map(|r| r.unwrap()).collect()
}

/// A sensible default worker count for I/O and hashing.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_order() {
        let v: Vec<u32> = (0..1000).collect();
        let out = par_map(&v, 8, |x| x * 2);
        assert_eq!(out, (0..1000).map(|x| x * 2).collect::<Vec<_>>());
        assert!(par_map::<u32, u32>(&[], 4, |x| *x).is_empty());
    }
}

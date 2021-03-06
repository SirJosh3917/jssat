//! Contains only the logic related to how the symbolic execution engine handles
//! the graph-theory part of symbolic execution. This allows the symbolic
//! execution engine to focus on handling symbolic execution, while behavior
//! regarding cyclic executions are handled by the graph system.

// TODO: use `stacker` for recursion needs

// TODO: this file was one beautiful, containing only the logic for the grpah system.
// now, it must contain additional code to handle panics, and is ugly.
#![allow(clippy::type_complexity)]

// TODO: there's heaves of unsafe here to support getting mutable access to
// workers after a panic occurs. there's also probably a lot of UB here too.
// HOWEVER, consider the following: it compiles :)

use std::{
    cell::UnsafeCell,
    hash::Hash,
    panic::{RefUnwindSafe, UnwindSafe},
    sync::{Arc, Mutex, TryLockError},
};

use rustc_hash::FxHashMap;

/// Implemented by nodes that are a part of the graph system. Provides an
/// interface for making requests to the graph system.
pub trait Worker {
    /// The unique ID of the worker, used in order for workers to locate other
    /// workers.
    type Id: Copy + Eq + Hash;

    /// The type of results produced by a worker node, which are consumed by
    /// other worker nodes.
    type Result;

    /// Performs the work required of this worker node. The node is given a
    /// reference to a system to use to make requests to other worker nodes.
    fn work(&mut self, system: &impl System<Self>) -> Computation<Self::Result>;
}

/// Contract that a system provides. This is used for the [`Worker::work`]
/// method so that workers can call upon the system to invoke other workers.
pub trait System<W>
where
    W: ?Sized + Worker,
{
    /// Given the ID of a worker, this function will execute that worker and
    /// produce the value that the worker executed.
    fn spawn(&self, id: W::Id) -> Arc<W::Result>;
}

/// The return type of a system, with the existence of a "bogus" state.
///
/// When a system enters a cyclic state, no worker is capable of making progress
/// as it depends upon the other workers to make progress. When this state is
/// reached, a bogus value is produced by the system. This bogus value can then
/// work through the system, and re-execute workers until a result is produced.
/// In addition, the system may re-execute nodes in order to refine the value
/// produced.
pub enum Computation<R> {
    Bogus,
    Result(R),
}

/// Given an ID, will produce a worker.
pub trait WorkerFactory {
    type Worker: Worker;

    fn make(&mut self, id: <Self::Worker as Worker>::Id) -> Self::Worker;
}

/// A graph system that performs arbitrary computations
#[derive(Clone)]
pub struct ComputeGraphSys<W: Worker, F> {
    inner: Arc<GraphSystemInner<W, F>>,
    callstack: Arc<CallStack<W, W::Id>>,
    global_callstack: Arc<Mutex<Arc<CallStack<W, W::Id>>>>,
}

#[derive(Debug)]
pub enum ResultsError {
    ReferencesToSystem,
    LockHeldOnWorkers,
    ReferencesToWorkerResults,
}

impl<W, F> ComputeGraphSys<W, F>
where
    W: Worker,
    F: WorkerFactory<Worker = W>,
{
    pub fn new(factory: F) -> Self {
        let callstack = Arc::new(CallStack::Root);

        ComputeGraphSys {
            inner: Arc::new(GraphSystemInner {
                workers: Mutex::default(),
                factory: Mutex::new(factory),
                current_callstack: Mutex::new(callstack.clone()),
            }),
            global_callstack: Arc::new(Mutex::new(callstack.clone())),
            callstack,
        }
    }

    pub fn try_into_results(self) -> Result<FxHashMap<W::Id, W::Result>, ResultsError> {
        let system_inner =
            Arc::try_unwrap(self.inner).map_err(|_| ResultsError::ReferencesToSystem)?;
        let workers =
            Mutex::into_inner(system_inner.workers).map_err(|_| ResultsError::LockHeldOnWorkers)?;

        let mut results = FxHashMap::default();
        for (id, state) in workers {
            let result = match state {
                WorkStatus::Working => {
                    panic!("system should be in finished state, yet workers remain working")
                }
                WorkStatus::Completed(r) => r,
            };

            let result =
                Arc::try_unwrap(result).map_err(|_| ResultsError::ReferencesToWorkerResults)?;
            results.insert(id, result);
        }
        Ok(results)
    }
}

impl<W, F> ComputeGraphSys<W, F>
where
    W: Worker + Send + Sync,
    W::Id: 'static + Send + Sync,
    F: WorkerFactory<Worker = W>,
{
    pub fn load_callstack_within_panic(&self) -> Vec<(<W as Worker>::Id, Arc<SuperUnsafeCell<W>>)> {
        let callstack = self.global_callstack.clone();

        let callstack = match callstack.try_lock() {
            Ok(c) => c,
            Err(TryLockError::Poisoned(g)) => g.into_inner(),
            Err(TryLockError::WouldBlock) => {
                panic!("wtf?");
            }
        };

        let mut frames = Vec::new();

        let mut current_frame = (**callstack).clone();

        while let CallStack::Child {
            level: _,
            previous,
            frame,
            worker,
        } = current_frame
        {
            frames.push((frame, worker));
            current_frame = (*previous).clone();
        }

        frames
    }
}

enum CallStack<W, F: Clone> {
    Root,
    Child {
        level: usize,
        previous: Arc<CallStack<W, F>>,
        frame: F,
        worker: Arc<SuperUnsafeCell<W>>,
    },
}

impl<W, F: Clone> Clone for CallStack<W, F> {
    fn clone(&self) -> Self {
        match self {
            Self::Root => Self::Root,
            Self::Child {
                level,
                previous,
                frame,
                worker,
            } => Self::Child {
                level: *level,
                previous: previous.clone(),
                frame: frame.clone(),
                worker: worker.clone(),
            },
        }
    }
}

pub struct SuperUnsafeCell<T>(pub UnsafeCell<T>);
unsafe impl<T> Send for SuperUnsafeCell<T> {}
unsafe impl<T> Sync for SuperUnsafeCell<T> {}
impl<T> UnwindSafe for SuperUnsafeCell<T> {}
impl<T> RefUnwindSafe for SuperUnsafeCell<T> {}
impl<T> SuperUnsafeCell<T> {
    /// # Safety
    ///
    /// This is not safe. Know what you're doing
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn raw_get(&self) -> &mut T {
        &mut *UnsafeCell::raw_get(&self.0 as *const UnsafeCell<T>)
    }
}

struct GraphSystemInner<W: Worker, F> {
    workers: Mutex<FxHashMap<W::Id, WorkStatus<W::Result>>>,
    factory: Mutex<F>,
    current_callstack: Mutex<Arc<CallStack<W, W::Id>>>,
}

enum WorkStatus<R> {
    Working,
    Completed(Arc<R>),
}

impl<W, F> System<W> for ComputeGraphSys<W, F>
where
    W: Worker,
    F: WorkerFactory<Worker = W>,
{
    fn spawn(&self, id: W::Id) -> Arc<W::Result> {
        GraphSystemInner::spawn(&self.inner, id, &self.callstack, &self.global_callstack)
    }
}

impl<W, F> GraphSystemInner<W, F>
where
    W: Worker,
    F: WorkerFactory<Worker = W>,
{
    fn spawn(
        me: &Arc<GraphSystemInner<W, F>>,
        id: W::Id,
        callstack: &Arc<CallStack<W, W::Id>>,
        global_callstack: &Arc<Mutex<Arc<CallStack<W, W::Id>>>>,
    ) -> Arc<W::Result> {
        match &**callstack {
            CallStack::Child { level, .. } if *level >= 1000 => {
                panic!("program appears to be deeply nested program - terminating early as a precaution")
            }
            _ => {}
        };

        let mut workers = me.workers.try_lock().expect("should be contentionless");

        // check if we've already executed this worker
        if let Some(worker) = workers.get(&id) {
            match &worker {
                WorkStatus::Working => {
                    // TODO: in order to support cyclic workers, not only must
                    // we produce a bogus value here, but we must know to
                    // re-execute workers until the bogus value is refined
                    panic!("cyclic workers not supported yet")
                }
                WorkStatus::Completed(r) => return r.clone(),
            };
        }

        // we've never executed a worker with this id
        // produce a new one
        let mut factory = me.factory.try_lock().expect("should be contentionless");
        let worker = factory.make(id);
        let worker = Arc::new(SuperUnsafeCell(UnsafeCell::new(worker)));
        drop(factory);

        // record this worker as working
        let state = WorkStatus::Working;
        workers.insert(id, state);

        // drop the lock so that the `work` call won't deadlock
        drop(workers);

        // update the callstack for panic info
        let callstack = Arc::new(CallStack::Child {
            level: match &**callstack {
                CallStack::Root => 1,
                CallStack::Child { level, .. } => *level + 1,
            },
            frame: id,
            previous: callstack.clone(),
            worker: worker.clone(),
        });
        {
            let mut cur_clstk = me
                .current_callstack
                .try_lock()
                .expect("should be contentionless");
            *cur_clstk = callstack.clone();

            let mut glb_clstk = global_callstack
                .try_lock()
                .expect("should be contentionless");
            *glb_clstk = callstack.clone();
        }

        // perform the work
        let sys_api = ComputeGraphSys {
            inner: me.clone(),
            callstack,
            global_callstack: global_callstack.clone(),
        };

        let worker = unsafe { worker.raw_get() };
        let result = match worker.work(&sys_api) {
            Computation::Result(result) => Arc::new(result),
            Computation::Bogus => panic!("workers that return bogus values aren't supported yet"),
        };

        // update the worker status
        let mut workers = me.workers.try_lock().expect("should be contentionless");
        let state = workers.get_mut(&id).unwrap();
        *state = WorkStatus::Completed(result.clone());

        result
    }
}

use crate::storage::types::KVStorageTrait;
pub use crate::tasks::cache::CacheService;
pub use crate::tasks::types::{TaskRecord, TaskTrait};

use flume::{bounded, Receiver, Sender};
use tokio::time::{sleep, Duration};
use tracing::{debug, info};

pub struct TaskService {
	pub task: Box<dyn TaskTrait>,
	db: Box<dyn KVStorageTrait>,
	pub cache: CacheService,
	//pubsub, probably redundant
	event_publisher: Sender<TaskRecord>,
	pub event_receiver: Receiver<TaskRecord>,
}

const FLUME_PUBSUB_MAX_EVENT_STACK: usize = 100;

// todo global generic state
impl TaskService {
	pub fn new(task: Box<dyn TaskTrait>, db: Box<dyn KVStorageTrait>) -> Self {
		let task_id = task.get_id();
		info!("Job created id={}", task_id);
		let cache = CacheService::new(task_id);

		let (event_publisher, event_receiver): (Sender<TaskRecord>, Receiver<TaskRecord>) =
			bounded(FLUME_PUBSUB_MAX_EVENT_STACK);

		TaskService { task, db, event_publisher, event_receiver, cache }
	}

	// run once
	pub async fn run(&mut self) {
		let task_id = self.task.get_id();
		let restored_state = self.db.get(task_id.as_str());

		match restored_state {
			Some(state) => {
				info!("Restored state={}", state);
				self.task.set_state_dump(&state.clone());
			},
			None => {
				debug!("No previous state found");
			},
		}

		self.index().await;
	}

	pub async fn index(&mut self) {
		// todo catch inner level errors
		loop {
			let n: Option<u64> = None;

			// todo must be dedicated field in the global state
			let from = self.task.get_state().records_total as u64;

			let records = self.task.run(Some(from), n).await;
			let _ = self.cache.append_cache(records).await;

			/*
			for r in records.iter() {
				self.event_publisher.send(r.clone());
			}
			*/

			let task_id = self.task.get_id();
			let task_state = self.task.get_state_dump();
			let _ = self.db.put(task_id.as_str(), task_state.as_str());

			let state = self.task.get_state();

			if state.is_finished == true {
				info!("Job id={} is finished", task_id);
				break;
			}
			// info!("batch received {} id=", task_id);

			let duration = self.task.get_sleep_interval();
			self.sleep(duration).await;
		}
	}

	pub async fn sleep(&self, duration: Duration) {
		sleep(duration).await;
	}
}

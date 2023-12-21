use error::AttTrError;
use futures::stream::iter;
use proto_buf::combiner::linear_combiner_client::LinearCombinerClient;
use proto_buf::common::Void;
use proto_buf::indexer::indexer_client::IndexerClient;
use proto_buf::indexer::{IndexerEvent, Query};
use proto_buf::transformer::transformer_server::{Transformer, TransformerServer};
use proto_buf::transformer::{TermBatch, TermObject};
use rocksdb::{WriteBatch, DB};
use schemas::status::EndorseCredential;
use schemas::SchemaType;
use serde_json::from_str;
use std::error::Error;
use term::Term;

use tonic::transport::Channel;
use tonic::{transport::Server, Request, Response, Status};

use crate::schemas::approve::AuditApproveSchema;
use crate::schemas::disapprove::AuditDisapproveSchema;
use crate::schemas::follow::FollowSchema;
use crate::schemas::IntoTerm;

mod did;
mod error;
mod schemas;
mod term;
mod utils;

const MAX_TERM_BATCH_SIZE: u32 = 1000;
const MAX_ATT_BATCH_SIZE: u32 = 1000;
const ATTESTATION_SOURCE_ADDRESS: &str = "0x1";
const AUDIT_APPROVE_SCHEMA_ID: &str = "0x2";
const AUDIT_DISAPPROVE_SCHEMA_ID: &str = "0x3";
const ENDORSE_SCHEMA_ID: &str = "0x4";

#[derive(Debug)]
struct TransformerService {
	indexer_channel: Channel,
	lt_channel: Channel,
	db: String,
}

impl TransformerService {
	fn new(
		indexer_channel: Channel, lt_channel: Channel, db_url: &str,
	) -> Result<Self, AttTrError> {
		let db = DB::open_default(db_url).map_err(|e| AttTrError::DbError(e))?;
		let checkpoint = db.get(b"checkpoint").map_err(|e| AttTrError::DbError(e))?;
		if let None = checkpoint {
			let count = 0u32.to_be_bytes();
			db.put(b"checkpoint", count).map_err(|e| AttTrError::DbError(e))?;
		}

		Ok(Self { indexer_channel, lt_channel, db: db_url.to_string() })
	}

	fn read_checkpoint(db: &DB) -> Result<u32, AttTrError> {
		let offset_bytes_opt = db.get(b"checkpoint").map_err(|e| AttTrError::DbError(e))?;
		let offset_bytes = offset_bytes_opt.map_or([0; 4], |x| {
			let mut bytes: [u8; 4] = [0; 4];
			bytes.copy_from_slice(&x);
			bytes
		});
		let offset = u32::from_be_bytes(offset_bytes);
		Ok(offset)
	}

	fn write_checkpoint(db: &DB, count: u32) -> Result<(), AttTrError> {
		db.put(b"checkpoint", count.to_be_bytes()).map_err(|e| AttTrError::DbError(e))?;
		Ok(())
	}

	fn read_terms(db: &DB, batch: TermBatch) -> Result<Vec<TermObject>, AttTrError> {
		let mut terms = Vec::new();
		for i in batch.start..batch.size {
			let id_bytes = i.to_be_bytes();
			let res_opt = db.get(id_bytes).map_err(|e| AttTrError::DbError(e))?;
			let res = res_opt.ok_or_else(|| AttTrError::NotFoundError)?;
			let term = Term::from_bytes(res)?;
			let term_obj: TermObject = term.into();
			terms.push(term_obj);
		}
		Ok(terms)
	}

	fn parse_event(event: IndexerEvent) -> Result<(u32, Term), AttTrError> {
		let schema_id = event.schema_id;
		let schema_type = SchemaType::from(schema_id);
		let term = match schema_type {
			SchemaType::Follow => {
				let parsed_att: FollowSchema =
					from_str(&event.schema_value).map_err(|_| AttTrError::ParseError)?;
				parsed_att.into_term()?
			},
			SchemaType::AuditApprove => {
				let parsed_att: AuditApproveSchema =
					from_str(&event.schema_value).map_err(|_| AttTrError::ParseError)?;
				parsed_att.into_term()?
			},
			SchemaType::AuditDisapprove => {
				let parsed_att: AuditDisapproveSchema =
					from_str(&event.schema_value).map_err(|_| AttTrError::ParseError)?;
				parsed_att.into_term()?
			},
			SchemaType::EndorseCredential => {
				let parsed_att: EndorseCredential =
					from_str(&event.schema_value).map_err(|_| AttTrError::ParseError)?;
				parsed_att.into_term()?
			},
		};
		println!("{:?}", term);

		Ok((event.id, term))
	}

	fn write_terms(db: &DB, terms: Vec<(u32, Term)>) -> Result<(), AttTrError> {
		let mut batch = WriteBatch::default();
		for (id, term) in terms {
			let term_bytes = term.into_bytes()?;
			let id = id.to_be_bytes();
			batch.put(id, term_bytes);
		}
		db.write(batch).map_err(|e| AttTrError::DbError(e))
	}
}

#[tonic::async_trait]
impl Transformer for TransformerService {
	async fn sync_indexer(&self, _: Request<Void>) -> Result<Response<Void>, Status> {
		let db = DB::open_default(self.db.clone())
			.map_err(|_| Status::internal("Failed to connect to DB"))?;

		let offset = 0;

		let indexer_query = Query {
			source_address: ATTESTATION_SOURCE_ADDRESS.to_owned(),
			schema_id: vec![
				AUDIT_APPROVE_SCHEMA_ID.to_owned(),
				AUDIT_DISAPPROVE_SCHEMA_ID.to_owned(),
				ENDORSE_SCHEMA_ID.to_owned(),
			],
			offset: 0,
			count: MAX_ATT_BATCH_SIZE,
		};

		let mut client = IndexerClient::new(self.indexer_channel.clone());
		let mut response = client.subscribe(indexer_query).await?.into_inner();
		let mut count = offset;
		let mut terms = Vec::new();
		// ResponseStream
		while let Ok(Some(res)) = response.message().await {
			println!("{:?}", res);
			assert!(res.id == count);
			let term =
				Self::parse_event(res).map_err(|_| Status::internal("Failed to parse event"))?;
			terms.push(term);
			count += 1;
		}

		Self::write_terms(&db, terms).map_err(|_| Status::internal("Failed to write terms"))?;
		Self::write_checkpoint(&db, count)
			.map_err(|_| Status::internal("Failed to write checkpoint"))?;

		Ok(Response::new(Void::default()))
	}

	async fn term_stream(&self, request: Request<TermBatch>) -> Result<Response<Void>, Status> {
		let inner = request.into_inner();
		if inner.size > MAX_TERM_BATCH_SIZE {
			return Result::Err(Status::invalid_argument(format!(
				"Batch size too big. Max size: {}",
				MAX_TERM_BATCH_SIZE
			)));
		}

		let db = DB::open_default(self.db.clone())
			.map_err(|_| Status::internal("Failed to connect to DB"))?;

		let terms =
			Self::read_terms(&db, inner).map_err(|_| Status::internal("Failed to read terms"))?;

		let mut client = LinearCombinerClient::new(self.lt_channel.clone());
		let res = client.sync_transformer(Request::new(iter(terms))).await?;

		Ok(res)
	}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
	let indexer_channel = Channel::from_static("http://localhost:50050").connect().await?;
	let lt_channel = Channel::from_static("http://localhost:50052").connect().await?;
	let db_url = "att-tr-storage";
	let tr_service = TransformerService::new(indexer_channel, lt_channel, db_url)?;

	let addr = "[::1]:50051".parse()?;
	Server::builder().add_service(TransformerServer::new(tr_service)).serve(addr).await?;
	Ok(())
}

#[cfg(test)]
mod test {
	use crate::schemas::follow::{FollowSchema, Scope};
	use crate::schemas::IntoTerm;
	use crate::TransformerService;
	use proto_buf::indexer::IndexerEvent;
	use proto_buf::transformer::{TermBatch, TermObject};
	use rocksdb::DB;
	use serde_json::to_string;

	#[test]
	fn should_write_read_checkpoint() {
		let db = DB::open_default("att-tr-checkpoint-test-storage").unwrap();
		TransformerService::write_checkpoint(&db, 15).unwrap();
		let checkpoint = TransformerService::read_checkpoint(&db).unwrap();
		assert_eq!(checkpoint, 15);
	}

	#[test]
	fn should_write_read_term() {
		let db = DB::open_default("att-tr-terms-test-storage").unwrap();

		let follow_schema = FollowSchema::new(
			"did:pkh:eth:90f8bf6a479f320ead074411a4b0e7944ea8c9c2".to_owned(),
			true,
			Scope::Auditor,
		);
		let indexed_event = IndexerEvent {
			id: 0,
			schema_id: 1,
			schema_value: to_string(&follow_schema).unwrap(),
			timestamp: 2397848,
		};
		let term = TransformerService::parse_event(indexed_event).unwrap();
		TransformerService::write_terms(&db, vec![term]).unwrap();

		let term_batch = TermBatch { start: 0, size: 1 };
		let terms = TransformerService::read_terms(&db, term_batch).unwrap();

		let term = follow_schema.into_term().unwrap();
		let term_obj: TermObject = term.into();
		assert_eq!(terms, vec![term_obj]);
	}
}

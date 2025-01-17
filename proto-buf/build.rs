use tonic_build::compile_protos;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	compile_protos("services/common.proto")?;
	compile_protos("services/indexer.proto")?;
	compile_protos("services/transformer.proto")?;
	compile_protos("services/combiner.proto")?;
	compile_protos("services/eigentrust.proto")?;
	Ok(())
}

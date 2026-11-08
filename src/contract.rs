//! The contract of recstrap and recfstab for `cargo xtask gen`: the JSON
//! they print (recstrap's event stream, recfstab's result, the shared error
//! document) and the reader of their output streams.

use anyhow::Result;
use tool_contract::Contract;

use crate::output::{self, ErrorPayload, FstabResult, StrapEvent};

pub fn export() -> Result<Contract> {
    let schemas = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/schemas");
    let mut c = Contract::new("recstrap", &["recstrap", "recfstab"]);
    c.schema::<StrapEvent>(schemas, "recstrap-event")?
        .schema::<FstabResult>(schemas, "recfstab-result")?
        .schema::<ErrorPayload>(schemas, "error")?;
    c.types_from::<StrapEvent>().types_from::<FstabResult>().types_from::<ErrorPayload>();
    c.constant("STRAP_SCHEMA_VERSION", "", &output::STRAP_SCHEMA_VERSION)?
        .constant("FSTAB_SCHEMA_VERSION", "", &output::FSTAB_SCHEMA_VERSION)?
        .constant("ERROR_SCHEMA_VERSION", "", &output::ERROR_SCHEMA_VERSION)?;
    c.raw(include_str!("contract/output.ts"));
    Ok(c)
}

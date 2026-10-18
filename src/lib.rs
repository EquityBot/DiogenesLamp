//! DiogenesLamp: browse any Yupoo catalog locally as one fast, searchable page.

pub mod age;
pub mod argparse;
pub mod cli;
pub mod covers;
pub mod dedup;
pub mod errors;
pub mod html;
pub mod http;
pub mod ingest;
pub mod ipaddr;
pub mod isodate;
pub mod links;
pub mod locking;
pub mod maintenance;
mod mimetable;
pub mod out;
pub mod parse;
pub mod pyerr;
pub mod pyjson;
pub mod pystr;
mod pytables;
pub mod pyurl;
pub mod schema;
pub mod scrape;
pub mod serve;
pub mod storage;
pub mod viewer;

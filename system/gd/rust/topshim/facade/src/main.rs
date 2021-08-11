//! Starts the facade services that allow us to test the Bluetooth stack

#[macro_use]
extern crate clap;
use clap::{App, Arg};

#[macro_use]
extern crate lazy_static;

use bt_topshim::btif;

use futures::channel::mpsc;
use futures::executor::block_on;
use futures::stream::StreamExt;
use grpcio::*;
use log::debug;
use nix::sys::signal;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

mod adapter_service;
mod media_service;

fn main() {
    let sigint = install_sigint();
    bt_common::init_logging();
    let rt = Arc::new(Runtime::new().unwrap());
    rt.block_on(async_main(Arc::clone(&rt), sigint));
}

async fn async_main(rt: Arc<Runtime>, mut sigint: mpsc::UnboundedReceiver<()>) {
    let matches = App::new("bluetooth_topshim_facade")
        .about("The bluetooth topshim stack, with testing facades enabled and exposed via gRPC.")
        .arg(Arg::with_name("grpc-port").long("grpc-port").default_value("8899").takes_value(true))
        .arg(Arg::with_name("rootcanal-port").long("rootcanal-port").takes_value(true))
        .arg(Arg::with_name("btsnoop").long("btsnoop").takes_value(true))
        .arg(Arg::with_name("btsnooz").long("btsnooz").takes_value(true))
        .arg(Arg::with_name("btconfig").long("btconfig").takes_value(true))
        .get_matches();

    let grpc_port = value_t!(matches, "grpc-port", u16).unwrap();
    let _rootcanal_port = value_t!(matches, "rootcanal-port", u16).ok();
    let env = Arc::new(Environment::new(2));

    let btif_intf = Arc::new(Mutex::new(btif::get_btinterface().unwrap()));

    let mut server = ServerBuilder::new(env)
        .register_service(adapter_service::AdapterServiceImpl::create(
            rt.clone(),
            btif_intf.clone(),
        ))
        .register_service(media_service::MediaServiceImpl::create(rt.clone(), btif_intf.clone()))
        .bind("0.0.0.0", grpc_port)
        .build()
        .unwrap();
    server.start();

    sigint.next().await;
    block_on(server.shutdown()).unwrap();
}

// TODO: remove as this is a temporary nix-based hack to catch SIGINT
fn install_sigint() -> mpsc::UnboundedReceiver<()> {
    let (tx, rx) = mpsc::unbounded();
    *SIGINT_TX.lock().unwrap() = Some(tx);

    let sig_action = signal::SigAction::new(
        signal::SigHandler::Handler(handle_sigint),
        signal::SaFlags::empty(),
        signal::SigSet::empty(),
    );
    unsafe {
        signal::sigaction(signal::SIGINT, &sig_action).unwrap();
    }

    rx
}

lazy_static! {
    static ref SIGINT_TX: Mutex<Option<mpsc::UnboundedSender<()>>> = Mutex::new(None);
}

extern "C" fn handle_sigint(_: i32) {
    let mut sigint_tx = SIGINT_TX.lock().unwrap();
    if let Some(tx) = &*sigint_tx {
        debug!("Stopping gRPC root server due to SIGINT");
        tx.unbounded_send(()).unwrap();
    }
    *sigint_tx = None;
}
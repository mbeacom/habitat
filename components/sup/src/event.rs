// Copyright (c) 2019 Chef Software Inc. and/or applicable contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Main interface for a stream of events the Supervisor can send out
//! in the course of its operations.
//!
//! Currently, the Supervisor is able to send events to a [NATS
//! Streaming][1] server. The `init_stream` function must be called
//! before sending events to initialize the publishing thread in the
//! background. Thereafter, you can pass "event" structs to the
//! `event` function, which will publish the event to the stream.
//!
//! All events are published under the "habitat" subject.
//!
//! [1]:https://github.com/nats-io/nats-streaming-server

mod types;

use self::types::{EventMessage,
                  ServiceStartedEvent,
                  ServiceStoppedEvent};
use crate::{error::Result,
            manager::service::Service};
use futures::{sync::{mpsc as futures_mpsc,
                     mpsc::UnboundedSender},
              Future,
              Stream};
use nitox::{commands::ConnectCommand,
            streaming::{client::NatsStreamingClient,
                        error::NatsStreamingError},
            NatsClient,
            NatsClientOptions};
use state::Container;
use std::{sync::{mpsc as std_mpsc,
                 Once},
          thread};
use tokio::{executor,
            runtime::current_thread::Runtime as ThreadRuntime};

static INIT: Once = Once::new();
lazy_static! {
    // TODO (CM): When const fn support lands in stable, we can ditch
    // this lazy_static call.

    /// Reference to the event stream.
    static ref EVENT_STREAM: Container = Container::new();
    /// Core information that is shared between all events.
    static ref EVENT_CORE: Container = Container::new();
}

/// Starts a new thread for sending events to a NATS Streaming
/// server. Stashes the handle to the stream, as well as the core
/// event information that will be a part of all events, in a global
/// static reference for access later.
pub fn init_stream(conn_info: EventConnectionInfo, event_core: EventCore) {
    INIT.call_once(|| {
            let event_stream = init_nats_stream(conn_info).expect("Could not start NATS thread");
            EVENT_STREAM.set(event_stream);
            EVENT_CORE.set(event_core);
        });
}

/// All the information needed to establish a connection to a NATS
/// Streaming server.
// TODO: This will change as we firm up what the interaction between
// Habitat and A2 looks like.
pub struct EventConnectionInfo {
    pub name:        String,
    pub verbose:     bool,
    pub cluster_uri: String,
    pub cluster_id:  String,
}

/// A collection of data that will be present in all events. Rather
/// than baking this into the structure of each event, we represent it
/// once and merge the information into the final rendered form of the
/// event.
///
/// This prevents us from having to thread information throughout the
/// system, just to get it to the places where the events are
/// generated (e.g., not all code has direct access to the
/// Supervisor's ID).
#[derive(Clone, Debug)]
pub struct EventCore {
    /// The unique identifier of the Supervisor sending the event.
    pub supervisor_id: String,
}

/// Send an event for the start of a Service.
pub fn service_started(service: &Service) {
    if stream_initialized() {
        publish(ServiceStartedEvent { service_metadata:    Some(service.to_service_metadata()),
                                      supervisor_metadata: None, });
    }
}

/// Send an event for the stop of a Service.
pub fn service_stopped(service: &Service) {
    if stream_initialized() {
        publish(ServiceStoppedEvent { service_metadata:    Some(service.to_service_metadata()),
                                      supervisor_metadata: None, });
    }
}

////////////////////////////////////////////////////////////////////////

/// Internal helper function to know whether or not to go to the trouble of
/// creating event structures. If the event stream hasn't been
/// initialized, then we shouldn't need to do anything.
fn stream_initialized() -> bool { EVENT_STREAM.try_get::<EventStream>().is_some() }

/// Publish an event. This is the main interface that client code will
/// use.
///
/// If `init_stream` has not been called already, this function will
/// be a no-op.
fn publish(mut event: impl EventMessage) {
    // TODO: incorporate the current timestamp into the rendered event
    // (which will require tweaks to the rendering logic, but we know
    // that'll need to be updated anyway).
    if let Some(e) = EVENT_STREAM.try_get::<EventStream>() {
        event.supervisor_metadata(EVENT_CORE.get::<EventCore>().to_supervisor_metadata());
        if let Ok(bytes) = event.to_bytes() {
            e.send(bytes);
        }
    }
}

/// A lightweight handle for the event stream. All events get to the
/// event stream through this.
struct EventStream(UnboundedSender<Vec<u8>>);

impl EventStream {
    /// Queues an event to be sent out.
    fn send(&self, event: Vec<u8>) {
        trace!("About to queue an event: {:?}", event);
        if let Err(e) = self.0.unbounded_send(event) {
            error!("Failed to queue event: {:?}", e);
        }
    }
}

////////////////////////////////////////////////////////////////////////

/// All messages are published under this subject.
const HABITAT_SUBJECT: &str = "habitat";

/// Defines default connection information for a NATS Streaming server
/// running on localhost.
// TODO: As we become clear on the interaction between Habitat and A2,
// this implementation *may* disappear. It's useful for testing and
// prototyping, though.
impl Default for EventConnectionInfo {
    fn default() -> Self {
        EventConnectionInfo { name:        String::from("habitat"),
                              verbose:     true,
                              cluster_uri: String::from("127.0.0.1:4223"),
                              cluster_id:  String::from("test-cluster"), }
    }
}

fn init_nats_stream(conn_info: EventConnectionInfo) -> Result<EventStream> {
    // TODO (CM): Investigate back-pressure scenarios
    let (event_tx, event_rx) = futures_mpsc::unbounded();
    let (sync_tx, sync_rx) = std_mpsc::sync_channel(0); // rendezvous channel

    // TODO (CM): We could theoretically create this future and spawn
    // it in the Supervisor's Tokio runtime, but there's currently a
    // bug: https://github.com/YellowInnovation/nitox/issues/24

    thread::Builder::new().name("events".to_string())
                          .spawn(move || {
                              let EventConnectionInfo { name,
                                                        verbose,
                                                        cluster_uri,
                                                        cluster_id, } = conn_info;

                              let cc = ConnectCommand::builder()
                // .user(Some("nats".to_string()))
                // .pass(Some("S3Cr3TP@5w0rD".to_string()))
                .name(Some(name))
                .verbose(verbose)
                .build()
                .unwrap();
                              let opts =
                                  NatsClientOptions::builder().connect_command(cc)
                                                              .cluster_uri(cluster_uri.as_str())
                                                              .build()
                                                              .unwrap();

                              let publisher = NatsClient::from_options(opts)
                .map_err(Into::<NatsStreamingError>::into)
                .and_then(|client| {
                    NatsStreamingClient::from(client)
                        .cluster_id(cluster_id)
                        .connect()
                })
                .map_err(|streaming_error| error!("{}", streaming_error))
                .and_then(move |client| {
                    sync_tx.send(()).expect("Couldn't synchronize!");
                    event_rx.for_each(move |event: Vec<u8>| {
                        let publish_event = client
                            .publish(HABITAT_SUBJECT.into(), event.into())
                            .map_err(|e| {
                                error!("Error publishing event: {:?}", e);
                            });
                        executor::spawn(publish_event);
                        Ok(())
                    })
                });

                              ThreadRuntime::new().expect("Couldn't create event stream runtime!")
                                                  .spawn(publisher)
                                                  .run()
                                                  .expect("something seriously wrong has occurred");
                          })
                          .expect("Couldn't start events thread!");

    sync_rx.recv()?; // TODO (CM): nicer error message
    Ok(EventStream(event_tx))
}

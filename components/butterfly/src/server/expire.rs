//! Periodically check membership rumors to automatically time out
//! `Suspect` rumors to `Confirmed`, and `Confirmed` rumors to
//! `Departed`. This also expires any rumors that have expiration dates.

use crate::server::{timing::Timing,
                    Server};
use chrono::offset::Utc;
use std::{thread,
          time::Duration};

const LOOP_DELAY_MS: u64 = 500;

pub struct Expire {
    pub server: Server,
    pub timing: Timing,
}

impl Expire {
    pub fn new(server: Server, timing: Timing) -> Expire { Expire { server, timing } }

    pub fn run(&self) {
        loop {
            self.server
                .member_list
                .members_expired_to_confirmed(self.timing.suspicion_timeout_duration());

            self.server
                .member_list
                .members_expired_to_departed(self.timing.departure_timeout_duration());

            // JB TODO: How does this work for members, since members aren't /quite/
            // the same kind of rumor
            let now = Utc::now();
            self.server.departure_store.purge_expired(now);
            self.server.election_store.purge_expired(now);
            self.server.update_store.purge_expired(now);
            self.server.service_store.purge_expired(now);
            self.server.service_config_store.purge_expired(now);
            self.server.service_file_store.purge_expired(now);

            thread::sleep(Duration::from_millis(LOOP_DELAY_MS));
        }
    }
}

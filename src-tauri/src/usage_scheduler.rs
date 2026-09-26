#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::{Context, Power};

    fn context(foreground: bool, power: Power) -> Context {
        Context {
            visible: foreground,
            focused: foreground,
            power,
            revision: 0,
        }
    }
    fn account(id: &str, selected: bool) -> AccountInfo {
        AccountInfo {
            id: id.into(),
            provider: crate::state::ProviderId::Codex,
            revision: 0,
            selected,
            logging_in: false,
        }
    }

    #[test]
    fn selected_and_inactive_accounts_use_distinct_power_and_visibility_budgets() {
        let mut schedule = Schedule::default();
        schedule.sync(&[account("selected", true), account("other", false)]);
        let tickets = schedule.begin(0, context(true, Power::Ac));
        assert_eq!(tickets.len(), 2);
        for ticket in tickets {
            assert!(schedule.finish(&ticket, 0, true, None));
        }
        assert!(schedule.begin(59, context(true, Power::Ac)).is_empty());
        assert_eq!(schedule.begin(60, context(true, Power::Ac)).len(), 1);
        assert!(schedule
            .begin(119, context(true, Power::Battery))
            .is_empty());
        assert_eq!(schedule.begin(120, context(true, Power::Battery)).len(), 0); // selected remains in flight
        schedule.sync(&[account("selected", true), account("other", false)]);
        assert_eq!(schedule.begin(600, context(true, Power::Ac)).len(), 1);
    }

    #[test]
    fn a_removed_or_reconnected_account_rejects_its_old_probe() {
        let mut schedule = Schedule::default();
        schedule.sync(&[account("old", true)]);
        let ticket = schedule.begin(0, context(true, Power::Ac)).remove(0);
        schedule.forget("old");
        schedule.sync(&[account("old", true)]);
        assert!(!schedule.finish(&ticket, 1, true, None));
        assert_eq!(schedule.begin(1, context(true, Power::Ac)).len(), 1);
    }

    #[test]
    fn failures_back_off_and_explicit_wake_is_bounded() {
        let mut schedule = Schedule::default();
        schedule.sync(&[account("account", true)]);
        let ticket = schedule.begin(0, context(true, Power::Ac)).remove(0);
        schedule.finish(&ticket, 1, false, None);
        assert!(schedule.begin(30, context(true, Power::Ac)).is_empty());
        schedule.request(Some(crate::state::ProviderId::Codex), 32);
        assert_eq!(schedule.begin(32, context(true, Power::Ac)).len(), 1);
    }

    #[test]
    fn login_pause_and_live_event_defer_a_probe_until_due_or_reset() {
        let mut schedule = Schedule::default();
        let mut entry = account("account", true);
        entry.logging_in = true;
        schedule.sync(&[entry.clone()]);
        assert!(schedule.begin(0, context(true, Power::Ac)).is_empty());
        entry.logging_in = false;
        schedule.sync(&[entry]);
        schedule.live("account", 10, Some(40));
        assert!(schedule.begin(39, context(true, Power::Ac)).is_empty());
        assert_eq!(schedule.begin(40, context(true, Power::Ac)).len(), 1);
    }

    #[test]
    fn login_invalidates_a_probe_that_started_before_credentials_change() {
        let mut schedule = Schedule::default();
        let mut entry = account("account", true);
        schedule.sync(&[entry.clone()]);
        let ticket = schedule.begin(0, context(true, Power::Ac)).remove(0);

        entry.logging_in = true;
        schedule.sync(&[entry]);
        assert!(!schedule.current(&ticket));
        assert!(!schedule.finish(&ticket, 1, true, None));
        assert!(schedule.begin(1, context(true, Power::Ac)).is_empty());
    }

    #[test]
    fn cancelled_or_failed_login_resumes_probes_without_a_revision_change() {
        for selected in [true, false] {
            let mut schedule = Schedule::default();
            let mut entry = account("account", selected);
            schedule.sync(&[entry.clone()]);
            let old = schedule.begin(0, context(true, Power::Ac)).remove(0);

            entry.logging_in = true;
            schedule.sync(&[entry.clone()]);
            assert!(schedule.begin(1, context(true, Power::Ac)).is_empty());
            // The worker discards its result during login, without calling finish.
            entry.logging_in = false;
            schedule.sync(&[entry]);
            let tickets = schedule.begin(2, context(true, Power::Ac));
            assert_eq!(tickets.len(), 1);
            assert!(!schedule.finish(&old, 3, true, None));
            assert!(schedule.current(&tickets[0]));
            assert!(schedule.finish(&tickets[0], 3, true, None));
        }
    }

    #[test]
    fn live_event_wins_over_an_older_in_flight_probe() {
        let mut schedule = Schedule::default();
        schedule.sync(&[account("account", true)]);
        let ticket = schedule.begin(0, context(true, Power::Ac)).remove(0);
        schedule.live("account", 10, None);
        assert!(!schedule.current(&ticket));
        assert!(!schedule.finish(&ticket, 11, true, None));
        assert!(schedule.begin(69, context(true, Power::Ac)).is_empty());
        assert_eq!(schedule.begin(70, context(true, Power::Ac)).len(), 1);
    }
}
// Per-account quota deadlines. All times are monotonic seconds since the watcher started;
// persisted `Agent.at` is never used as a scheduling clock.

use crate::background::{Context, Power};
use crate::state::ProviderId;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct AccountInfo {
    pub id: String,
    pub provider: ProviderId,
    pub revision: u64,
    pub selected: bool,
    pub logging_in: bool,
}

#[derive(Clone)]
pub struct Ticket {
    pub id: String,
    pub generation: u64,
}

struct Entry {
    info: AccountInfo,
    generation: u64,
    last_attempt: Option<u64>,
    retry_due: u64,
    reset_due: Option<u64>,
    failures: usize,
    in_flight: bool,
    forced: bool,
}

#[derive(Default)]
pub struct Schedule {
    entries: BTreeMap<String, Entry>,
    serial: u64,
}

impl Schedule {
    pub fn sync(&mut self, accounts: &[AccountInfo]) {
        self.entries
            .retain(|id, _| accounts.iter().any(|account| &account.id == id));
        for account in accounts {
            let changed = self.entries.get(&account.id).is_none_or(|old| {
                old.info.revision != account.revision
                    || old.info.provider != account.provider
                    || old.info.logging_in != account.logging_in
            });
            if changed {
                self.serial += 1;
                self.entries.insert(
                    account.id.clone(),
                    Entry {
                        info: account.clone(),
                        generation: self.serial,
                        last_attempt: None,
                        retry_due: 0,
                        reset_due: None,
                        failures: 0,
                        in_flight: false,
                        forced: false,
                    },
                );
            } else if let Some(entry) = self.entries.get_mut(&account.id) {
                entry.info = account.clone();
            }
        }
    }

    pub fn forget(&mut self, id: &str) {
        self.entries.remove(id);
    }

    fn interval(entry: &Entry, context: Context) -> u64 {
        if !(context.visible && context.focused) {
            return if entry.info.selected { 900 } else { 1800 };
        }
        match (entry.info.selected, context.power == Power::Ac) {
            (true, true) => 60,
            (true, false) => 120,
            (false, true) => 600,
            (false, false) => 1200,
        }
    }

    fn due_at(entry: &Entry, context: Context) -> u64 {
        if entry.forced {
            return 0;
        }
        let Some(last) = entry.last_attempt else {
            return 0;
        };
        if entry.failures != 0 {
            return entry.retry_due;
        }
        let regular = last.saturating_add(Self::interval(entry, context));
        if entry.info.selected && context.visible && context.focused {
            regular.min(entry.reset_due.unwrap_or(u64::MAX))
        } else {
            regular
        }
    }

    pub fn begin(&mut self, now: u64, context: Context) -> Vec<Ticket> {
        let mut tickets = Vec::new();
        for entry in self.entries.values_mut() {
            if entry.info.logging_in || entry.in_flight || Self::due_at(entry, context) > now {
                continue;
            }
            entry.in_flight = true;
            entry.forced = false;
            tickets.push(Ticket {
                id: entry.info.id.clone(),
                generation: entry.generation,
            });
        }
        tickets
    }

    pub fn current(&self, ticket: &Ticket) -> bool {
        self.entries
            .get(&ticket.id)
            .is_some_and(|entry| entry.generation == ticket.generation && entry.in_flight)
    }

    pub fn finish(
        &mut self,
        ticket: &Ticket,
        now: u64,
        success: bool,
        reset_due: Option<u64>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&ticket.id) else {
            return false;
        };
        if entry.generation != ticket.generation || !entry.in_flight {
            return false;
        }
        entry.in_flight = false;
        entry.last_attempt = Some(now);
        if success {
            entry.failures = 0;
            entry.reset_due = reset_due;
        } else {
            entry.failures = (entry.failures + 1).min(5);
            let minutes = [1, 2, 5, 15, 30][entry.failures - 1];
            entry.retry_due = now.saturating_add(minutes * 60);
        }
        true
    }

    pub fn live(&mut self, id: &str, now: u64, reset_due: Option<u64>) {
        let Some(entry) = self.entries.get_mut(id) else {
            return;
        };
        self.serial += 1;
        entry.generation = self.serial;
        entry.in_flight = false;
        entry.last_attempt = Some(now);
        entry.failures = 0;
        entry.reset_due = reset_due;
        entry.forced = false;
    }

    pub fn request(&mut self, provider: Option<ProviderId>, now: u64) {
        for entry in self.entries.values_mut() {
            if !entry.info.selected
                || provider.is_some_and(|p| p != entry.info.provider)
                || entry.in_flight
                || entry.info.logging_in
            {
                continue;
            }
            if entry
                .last_attempt
                .is_none_or(|last| now.saturating_sub(last) >= 30)
            {
                entry.forced = true;
            }
        }
    }

    pub fn next_delay(&self, now: u64, context: Context) -> u64 {
        self.entries
            .values()
            .filter(|entry| !entry.info.logging_in && !entry.in_flight)
            .map(|entry| Self::due_at(entry, context).saturating_sub(now))
            .min()
            .unwrap_or(60)
            .clamp(1, 60)
    }
}

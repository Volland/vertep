//! The mediator against the `tsp-mediator` spec: routing, registration, durable
//! store-and-forward, pickup, live sessions, quotas and retention, metadata minimisation.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::*;
use vertep_federation::live::live_uri;
use vertep_federation::tsp_mailbox::MailboxError;
use vertep_federation::{NetError, PickupSource, RelayNetwork};
use vertep_store::{JournalStore, MemoryStore, Position};
use vertep_tsp_mediator::store::mailbox_iri;
use vertep_tsp_mediator::{Limits, MediatorOptions};

/// A mediator, a recipient B registered with it (offline: nothing serves its own endpoint) and
/// a sender A.
struct Setup {
    w: World,
    m: MediatorHandle,
    a: Party,
    b: Party,
    client: Arc<vertep_federation::tsp_mailbox::TspMailboxClient>,
}

fn setup_with(options: MediatorOptions) -> Setup {
    let w = World::new();
    let m = w.mediator_on(M1, Arc::new(MemoryStore::new()), options);
    let a = w.party("https://a.example/tsp", &[]);
    let b = w.party("https://b.example/tsp", &[m.did()]);
    let client = w.client(&m.did());
    client.register(b.session.clone()).expect("registered");
    Setup { w, m, a, b, client }
}

fn setup() -> Setup {
    setup_with(MediatorOptions::default())
}

impl Setup {
    fn send(&self, id: &str) -> Result<Vec<u8>, NetError> {
        let m = self.m.did();
        self.w.send_routed(&self.a, &self.b, &[m.as_str()], M1, id)
    }

    fn waiting(&self) -> u64 {
        self.client.status(self.b.did()).unwrap().message_count
    }
}

// @tg: verifies:: [[openspec:tsp-mediator#TSP Routed-Mode Intermediary#Middle hop]] {level: integration}
#[test]
fn a_middle_hop_forwards_without_itself_in_the_route() {
    let w = World::new();
    let m1 = w.mediator(M1);
    let m2 = w.mediator(M2);
    let a = w.party("https://a.example/tsp", &[]);
    let b = w.party("https://b.example/tsp", &[m1.did(), m2.did()]);
    w.client(&m2.did()).register(b.session.clone()).unwrap();
    w.send_routed(&a, &b, &[m1.did().as_str(), m2.did().as_str()], M1, "o-1")
        .expect("accepted");
    assert_eq!(m1.m.metrics().forwarded, 1, "M1 forwarded");
    assert_eq!(m1.m.metrics().stored, 0, "M1 kept nothing");
    // What M1 posted to M2 was a routed message addressed to M2 alone.
    let to_m2: Vec<Vec<u8>> = w
        .net
        .posts()
        .into_iter()
        .filter(|(u, _)| u == M2)
        .map(|(_, b)| b)
        .collect();
    // B's registration posted to M2 twice (relationship, register); then M1's forward.
    assert_eq!(to_m2.len(), 3);
    let (sender, receiver) =
        vertep_federation::tsp_mailbox::envelope_ids(to_m2.last().unwrap()).unwrap();
    assert_eq!(sender, m1.did(), "sent by M1");
    assert_eq!(receiver, m2.did());
    let held = m2.m.store().held(b.did(), 10);
    assert_eq!(held.len(), 1, "M2, the last hop, stored it for B");
    let (sender, env) = b.open(&w, &held[0].data);
    assert_eq!(sender, a.did());
    assert_eq!(env.message_id, "o-1");
}

// @tg: verifies:: [[openspec:tsp-mediator#TSP Routed-Mode Intermediary#Last hop to an online recipient]] {level: integration}
#[test]
fn the_last_hop_delivers_to_an_online_recipient_and_keeps_no_copy() {
    let s = setup();
    let inbox = Arc::new(Inbox::default());
    s.w.net.serve(&s.b.uri, inbox.clone());
    s.send("o-2").expect("accepted");
    let got = inbox.0.lock().unwrap().clone();
    assert_eq!(got.len(), 1, "delivered to B's endpoint");
    let (sender, env) = s.b.open(&s.w, &got[0]);
    assert_eq!(
        (sender.as_str(), env.message_id.as_str()),
        (s.a.did(), "o-2")
    );
    assert_eq!(s.waiting(), 0, "no copy kept");
    assert_eq!(s.m.m.metrics().delivered_direct, 1);
}

// @tg: verifies:: [[openspec:tsp-mediator#Mailbox Registration#Register a mailbox]] {level: integration}
#[test]
fn registration_returns_the_routing_to_publish() {
    let w = World::new();
    let m = w.mediator(M1);
    let b = w.party("https://b.example/tsp", &[m.did()]);
    let reg = w.client(&m.did()).register(b.session.clone()).unwrap();
    assert_eq!(reg.recipient, b.did());
    assert_eq!(reg.mediator, m.did());
    assert_eq!(reg.endpoint, M1);
    assert_eq!(reg.routing, vec![m.did()]);
    let svc = reg.service("https://b.example/tsp");
    assert_eq!(svc.endpoints[0].routing_keys, vec![m.did()]);
    assert!(m.m.store().is_registered(b.did()));
}

// @tg: verifies:: [[openspec:tsp-mediator#Mailbox Registration#Unregistered recipient refused]] {level: integration}
#[test]
fn an_unregistered_unreachable_recipient_is_refused() {
    let w = World::new();
    let m = w.mediator(M1);
    let a = w.party("https://a.example/tsp", &[]);
    let c = w.party("https://c.example/tsp", &[m.did()]);
    let r = w.send_routed(&a, &c, &[m.did().as_str()], M1, "o-3");
    assert!(
        matches!(r, Err(NetError::Refused(ref e)) if e.contains("no mailbox")),
        "{r:?}"
    );
    assert_eq!(m.m.metrics().stored, 0);
}

#[test]
fn registration_is_revocable_by_the_vid_and_the_operator() {
    let s = setup();
    s.send("o-4").unwrap();
    s.client.unregister(s.b.did()).unwrap();
    assert!(!s.m.m.store().is_registered(s.b.did()));
    assert!(s.send("o-5").is_err(), "no mailbox any more");
    // Register again, then the operator revokes it.
    s.client.register(s.b.session.clone()).unwrap();
    s.m.m.revoke(s.b.did()).unwrap();
    assert!(matches!(
        s.client.register(s.b.session.clone()),
        Err(MailboxError::Problem { code, .. }) if code == "revoked"
    ));
}

// @tg: verifies:: [[openspec:tsp-mediator#Durable Store-And-Forward#Recipient offline]] {level: integration}
#[test]
fn a_message_for_an_offline_recipient_is_stored_before_it_is_accepted() {
    let s = setup();
    assert_eq!(s.send("o-6").expect("accepted"), Vec::<u8>::new());
    assert_eq!(s.waiting(), 1);
    // The acceptance followed the append: the journal holds the stored entry.
    let head = s.m.journal.head(&mailbox_iri(s.b.did())).unwrap();
    assert!(
        head >= Position::new(2),
        "register and stored are journaled"
    );
}

// @tg: verifies:: [[openspec:tsp-mediator#Durable Store-And-Forward#Mediator restarts]] {level: integration}
#[test]
fn stored_messages_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let w = World::new();
    let journal: Arc<dyn JournalStore> =
        Arc::new(vertep_tiramemsu::TiramemsuBackend::open(dir.path(), 2).unwrap());
    let m = w.mediator_on(M1, journal, MediatorOptions::default());
    let a = w.party("https://a.example/tsp", &[]);
    let b = w.party("https://b.example/tsp", &[m.did()]);
    let client = w.client(&m.did());
    client.register(b.session.clone()).unwrap();
    for i in 0..100 {
        w.send_routed(&a, &b, &[m.did().as_str()], M1, &format!("r-{i}"))
            .unwrap();
    }
    let (identity, km) = (m.identity.clone(), m.km.clone());
    drop(m);
    w.net.remove(M1);
    w.live.remove(&live_uri(M1));
    let journal: Arc<dyn JournalStore> =
        Arc::new(vertep_tiramemsu::TiramemsuBackend::open(dir.path(), 2).unwrap());
    let restarted = w.mediator_with(M1, identity, km, journal, MediatorOptions::default());
    assert_eq!(client.status(b.did()).unwrap().message_count, 100);
    let batch = client.deliver(b.did(), 100).unwrap();
    assert_eq!(batch.messages.len(), 100);
    let (_, env) = b.open(&w, &batch.messages[0].data);
    assert_eq!(env.message_id, "r-0");
    drop(restarted);
}

// @tg: verifies:: [[openspec:tsp-mediator#Durable Store-And-Forward#Store fails]] {level: integration}
#[test]
fn a_failed_store_refuses_the_message_for_retry() {
    let w = World::new();
    let flaky = Flaky::new();
    let m = w.mediator_on(M1, flaky.clone(), MediatorOptions::default());
    let a = w.party("https://a.example/tsp", &[]);
    let b = w.party("https://b.example/tsp", &[m.did()]);
    let client = w.client(&m.did());
    client.register(b.session.clone()).unwrap();
    flaky.fail.store(true, Ordering::SeqCst);
    let r = w.send_routed(&a, &b, &[m.did().as_str()], M1, "o-7");
    assert!(
        matches!(r, Err(NetError::Unreachable(_))),
        "the previous hop keeps it: {r:?}"
    );
    flaky.fail.store(false, Ordering::SeqCst);
    assert_eq!(
        client.status(b.did()).unwrap().message_count,
        0,
        "nothing stored"
    );
    w.send_routed(&a, &b, &[m.did().as_str()], M1, "o-7")
        .unwrap();
    assert_eq!(
        client.status(b.did()).unwrap().message_count,
        1,
        "the retry is stored"
    );
}

// @tg: verifies:: [[openspec:tsp-mediator#Pickup Protocol#Batch pickup]] {level: integration}
#[test]
fn a_batch_is_not_an_acknowledgement() {
    let s = setup();
    for i in 0..3 {
        s.send(&format!("b-{i}")).unwrap();
    }
    let batch = s.client.deliver(s.b.did(), 10).unwrap();
    assert_eq!(batch.messages.len(), 3);
    assert_eq!(batch.remaining, 0);
    assert_eq!(s.waiting(), 3, "still held after delivery");
    let ids: Vec<String> = batch.messages.iter().map(|m| m.id.clone()).collect();
    s.client.received(s.b.did(), &ids).unwrap();
    assert_eq!(s.waiting(), 0, "deleted on received");
}

// @tg: verifies:: [[openspec:tsp-mediator#Pickup Protocol#Lost acknowledgement]] {level: integration}
#[test]
fn a_lost_acknowledgement_means_delivery_again() {
    let s = setup();
    s.send("l-1").unwrap();
    s.send("l-2").unwrap();
    let first = s.client.deliver(s.b.did(), 10).unwrap();
    // The recipient stored both, but its `received` never arrived.
    let second = s.client.deliver(s.b.did(), 10).unwrap();
    assert_eq!(first.messages, second.messages);
}

// @tg: verifies:: [[openspec:tsp-mediator#Pickup Protocol#Only the owner picks up]] {level: integration}
#[test]
fn only_the_recipient_picks_up_its_messages() {
    let s = setup();
    s.send("p-1").unwrap();
    let eve = s.w.party("https://eve.example/tsp", &[s.m.did()]);
    s.client.register(eve.session.clone()).unwrap();
    let r = s.client.deliver_for(eve.did(), s.b.did(), 10);
    assert!(
        matches!(r, Err(MailboxError::Problem { ref code, .. }) if code == "not-owner"),
        "{r:?}"
    );
    // Eve's own pickup returns her (empty) mailbox, never B's messages.
    assert!(s.client.deliver(eve.did(), 10).unwrap().messages.is_empty());
    assert_eq!(s.waiting(), 1);
}

// @tg: verifies:: [[openspec:tsp-mediator#Live Delivery Session#Push while connected]] {level: integration}
#[test]
fn a_live_session_gets_messages_pushed() {
    let s = setup();
    let woken = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let wk = woken.clone();
    assert!(s.client.live(
        &[s.b.did().to_owned()],
        Arc::new(move |r: &str| wk.lock().unwrap().push(r.to_owned()))
    ));
    assert_eq!(s.w.live.open_connections(&live_uri(M1)), 1);
    s.send("live-1").unwrap();
    assert_eq!(woken.lock().unwrap().as_slice(), &[s.b.did().to_owned()]);
    // The pushed message is fetched without another request to the mediator.
    let posts_before = s.w.net.posts_to(M1);
    let got = s.client.fetch(&[s.b.did().to_owned()], 10).unwrap();
    assert_eq!(
        s.w.net.posts_to(M1),
        posts_before,
        "no request to the mediator"
    );
    assert_eq!(got.len(), 1);
    let (_, env) = s.b.open(&s.w, &got[0].bytes);
    assert_eq!(env.message_id, "live-1");
    assert_eq!(s.m.m.metrics().pushed, 1);
    // Still held until acknowledged; the acknowledgement goes over the live session.
    assert_eq!(s.m.m.store().held(s.b.did(), 10).len(), 1);
    let posts_before = s.w.net.posts_to(M1);
    s.client.ack(&[got[0].id.clone()]).unwrap();
    assert_eq!(
        s.w.net.posts_to(M1),
        posts_before,
        "acknowledged on the session"
    );
    assert!(s.m.m.store().held(s.b.did(), 10).is_empty());
}

// @tg: verifies:: [[openspec:tsp-mediator#Live Delivery Session#Connection drops before acknowledgement]] {level: integration}
#[test]
fn a_dropped_session_loses_nothing() {
    let s = setup();
    let wake: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|_: &str| {});
    s.client.live(&[s.b.did().to_owned()], wake.clone());
    s.send("d-1").unwrap();
    s.w.live.drop_connections(&live_uri(M1));
    assert!(!s.client.is_live(s.b.did()));
    assert_eq!(
        s.m.m.store().held(s.b.did(), 10).len(),
        1,
        "still at the mediator"
    );
    // A new session gets it pushed again.
    let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wk = woken.clone();
    s.client.live(
        &[s.b.did().to_owned()],
        Arc::new(move |_: &str| {
            wk.fetch_add(1, Ordering::SeqCst);
        }),
    );
    let got = s.client.fetch(&[s.b.did().to_owned()], 10).unwrap();
    assert_eq!(got.len(), 1, "delivered again, once");
    assert_eq!(s.b.open(&s.w, &got[0].bytes).1.message_id, "d-1");
}

// @tg: verifies:: [[openspec:tsp-mediator#Retention And Quotas#Mailbox full]] {level: integration}
#[test]
fn a_full_mailbox_refuses_and_keeps_what_it_holds() {
    let s = setup_with(MediatorOptions {
        limits: Limits {
            max_messages: 2,
            ..Limits::default()
        },
        ..MediatorOptions::default()
    });
    s.send("f-1").unwrap();
    s.send("f-2").unwrap();
    let r = s.send("f-3");
    assert!(
        matches!(r, Err(NetError::Refused(ref e)) if e.contains("full")),
        "{r:?}"
    );
    let held = s.client.deliver(s.b.did(), 10).unwrap();
    let ids: Vec<String> = held
        .messages
        .iter()
        .map(|m| s.b.open(&s.w, &m.data).1.message_id)
        .collect();
    assert_eq!(ids, vec!["f-1", "f-2"], "nothing dropped");
    assert_eq!(s.m.m.metrics().mailboxes[0].refusals, 1);
}

#[test]
fn a_mailbox_over_its_size_limit_refuses() {
    let s = setup_with(MediatorOptions {
        limits: Limits {
            max_bytes: 10,
            ..Limits::default()
        },
        ..MediatorOptions::default()
    });
    assert!(matches!(s.send("big"), Err(NetError::Refused(_))));
}

// @tg: verifies:: [[openspec:tsp-mediator#Retention And Quotas#Retention expiry reported]] {level: integration}
#[test]
fn expiry_is_reported_on_the_next_pickup() {
    let s = setup_with(MediatorOptions {
        limits: Limits {
            retention_ms: 60_000,
            ..Limits::default()
        },
        ..MediatorOptions::default()
    });
    s.send("e-1").unwrap();
    s.w.clock.advance(Duration::from_secs(30));
    s.send("e-2").unwrap();
    s.w.clock.advance(Duration::from_secs(40));
    let batch = s.client.deliver(s.b.did(), 10).unwrap();
    assert_eq!(batch.messages.len(), 1, "e-1 expired");
    assert_eq!(batch.expired.count, 1);
    assert_eq!(batch.expired.oldest_expiry_ms, T0 + 70_000);
    // Reported once.
    assert_eq!(s.client.status(s.b.did()).unwrap().expired.count, 0);
    assert_eq!(s.client.expired(s.b.did()).count, 1);
}

// @tg: verifies:: [[openspec:tsp-mediator#Metadata Minimisation#Store inspected]] {level: integration}
#[test]
fn the_store_holds_no_event_or_inner_sender() {
    let s = setup();
    s.send("secret-order-77").unwrap();
    let dump = format!(
        "{:?}",
        s.m.journal
            .read(&mailbox_iri(s.b.did()), Position::ZERO)
            .unwrap()
    );
    for forbidden in ["order.placed", "secret-order-77", "s-secret", s.a.did()] {
        assert!(!dump.contains(forbidden), "store reveals {forbidden}");
    }
    // What is there: the recipient, the sealed bytes, an id, arrival and size.
    assert!(dump.contains(s.b.did()));
    for field in ["messageId", "data", "arrival", "size"] {
        assert!(dump.contains(field), "{field} is stored");
    }
}

#[test]
fn metrics_render_as_prometheus_text() {
    let s = setup();
    s.send("x-1").unwrap();
    let text = s.m.m.metrics().to_prometheus();
    assert!(text.contains("vt_mediator_stored_total 1"));
    assert!(text.contains(&format!(
        "vt_mediator_mailbox_waiting{{vid=\"{}\"}} 1",
        s.b.did()
    )));
}

#[test]
fn a_duplicate_post_is_stored_once() {
    let s = setup();
    let m = s.m.did();
    let hops = [s.w.vid(&m)];
    let sealed =
        s.a.session
            .seal_routed(&s.w.vid(s.b.did()), &hops, &envelope("dup"))
            .unwrap();
    s.w.net.post(M1, &sealed.bytes).unwrap();
    s.w.net.post(M1, &sealed.bytes).unwrap();
    assert_eq!(s.waiting(), 1);
}

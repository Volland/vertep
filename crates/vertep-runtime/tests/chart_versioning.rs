//! Scenarios of the `chart-versioning` capability.
mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{
    ActivationError, Address, MigrateError, Migration, Node, NodeConfig, RemoveError, VirtualClock,
};
use vertep_store::{GraphStore, MemoryStore};

const V1: &str = r##"<state id="Idle"><transition event="pay" target="Paying"/></state>
  <state id="Paying" initial="Paying.Card"><state id="Paying.Card"><transition event="ok" target="Done"/></state></state>
  <state id="Done"><transition event="*"/></state>"##;

/// Like V1 with the card state renamed.
const V2_RENAMED: &str = r##"<state id="Idle"><transition event="pay" target="Paying"/></state>
  <state id="Paying" initial="Paying.Card2"><state id="Paying.Card2"><transition event="ok" target="Done"/></state></state>
  <state id="Done"><transition event="*"/></state>"##;

/// Like V1 with different executable content only.
const V2_SAME: &str = r##"<state id="Idle"><onentry><log label="v2"/></onentry><transition event="pay" target="Paying"/></state>
  <state id="Paying" initial="Paying.Card"><state id="Paying.Card"><transition event="ok" target="Done"/></state></state>
  <state id="Done"><transition event="*"/></state>"##;

fn versions(node: &Node, a: &Address) -> Vec<String> {
    node.journal(a)
        .unwrap()
        .iter()
        .map(|e| e.version.clone())
        .collect()
}

// @tg: verifies:: [[openspec:chart-versioning#Version Recorded Per Journal Segment#Version on every entry]] {level: integration}
#[test]
fn version_on_every_entry() {
    let (mut node, _clock, _store) = fresh();
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    let a = va("pay", "a");
    for e in ["pay", "ok", "x"] {
        node.send(&a, ev(e)).unwrap();
    }
    node.run_until_idle();
    assert_eq!(versions(&node, &a), vec![v1.as_str().to_owned(); 3]);
    assert!(v1.as_str().starts_with("urn:vertep:chart:sha256:"));
}

// @tg: verifies:: [[openspec:chart-versioning#Version Recorded Per Journal Segment#Same chart from different surfaces]] {level: integration}
#[test]
fn same_chart_from_different_surfaces() {
    let from_scxml = scxml("Idle", V1);
    let yaml = vertep_yaml::write(&from_scxml).unwrap();
    let from_yaml = vertep_yaml::read_str(&yaml).unwrap();
    let v_scxml = vertep_rdf::chart_version(&from_scxml);
    let v_yaml = vertep_rdf::chart_version(&from_yaml);
    assert_eq!(v_scxml, v_yaml);
    // a journal written under the SCXML load replays under the YAML load
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut n1 = node_on(store.clone(), &clock, NodeConfig::default());
    n1.register_type(&ty("pay"), from_scxml).unwrap();
    let a = va("pay", "a");
    n1.send(&a, ev("pay")).unwrap();
    n1.run_until_idle();
    let mut n2 = node_on(store, &clock, NodeConfig::default());
    n2.register_type(&ty("pay"), from_yaml).unwrap();
    n2.send(&a, ev("ok")).unwrap();
    n2.run_until_idle();
    assert_eq!(n2.state(&a).unwrap().configuration, vec!["Done"]);
}

fn with_migration(node: &mut Node, a: &Address) -> (String, String) {
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    node.send(a, ev("pay")).unwrap();
    node.run_until_idle();
    let v2 = node
        .register_type(&ty("pay"), scxml("Idle", V2_RENAMED))
        .unwrap();
    node.register_migration(
        Migration::identity("urn:vertep:migration:card2", v1.clone(), v2.clone())
            .rename_state("Paying.Card", "Paying.Card2"),
    );
    (v1.as_str().to_owned(), v2.as_str().to_owned())
}

// @tg: verifies:: [[openspec:chart-versioning#Replay With Recorded Version#Old segment replayed with old chart]] {level: integration}
#[test]
fn old_segment_replayed_with_old_chart() {
    let (mut node, _clock, _store) = fresh();
    let a = va("pay", "a");
    let (v1, v2) = with_migration(&mut node, &a);
    node.migrate(&a, "urn:vertep:migration:card2").unwrap();
    node.send(&a, ev("ok")).unwrap();
    node.run_until_idle();
    assert_eq!(versions(&node, &a), vec![v1, v2.clone(), v2]);
    node.deactivate(&a);
    let replayed = node.stats().replayed;
    node.activate_actor(&a).unwrap();
    assert_eq!(
        node.stats().replayed - replayed,
        2,
        "v1 entry, migration loaded, v2 entry"
    );
    assert_eq!(node.state(&a).unwrap().configuration, vec!["Done"]);
}

// @tg: verifies:: [[openspec:chart-versioning#Replay With Recorded Version#Missing version]] {level: integration}
#[test]
fn missing_version() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut n1 = node_on(store.clone(), &clock, NodeConfig::default());
    let v1 = n1.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    let a = va("pay", "a");
    n1.send(&a, ev("pay")).unwrap();
    n1.run_until_idle();
    let mut n2 = node_on(store, &clock, NodeConfig::default());
    n2.register_type(&ty("pay"), scxml("Idle", V2_SAME))
        .unwrap();
    let err = n2.activate_actor(&a).unwrap_err();
    assert_eq!(
        err,
        ActivationError::MissingVersion {
            actor: a.clone(),
            version: v1.as_str().to_owned()
        }
    );
    assert!(err.to_string().contains(v1.as_str()));
    assert!(!n2.is_active(&a));
}

// @tg: verifies:: [[openspec:chart-versioning#Chart Registry#Registering a new version]] {level: integration}
#[test]
fn registering_a_new_version() {
    let (mut node, _clock, _store) = fresh();
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    let v2 = node
        .register_type(&ty("pay"), scxml("Idle", V2_SAME))
        .unwrap();
    assert_ne!(v1, v2);
    assert!(node.chart(v1.as_str()).is_some(), "the old version is kept");
    assert!(node.chart(v2.as_str()).is_some());
    assert_eq!(node.latest(&ty("pay")).unwrap().version, v2);
}

// @tg: verifies:: [[openspec:chart-versioning#Chart Registry#Removal refused]] {level: integration}
#[test]
fn removal_refused() {
    let (mut node, _clock, _store) = fresh();
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    let a = va("pay", "a");
    node.send(&a, ev("pay")).unwrap();
    node.run_until_idle();
    node.register_type(&ty("pay"), scxml("Idle", V2_SAME))
        .unwrap();
    assert_eq!(
        node.remove_version(v1.as_str()),
        Err(RemoveError::InUse(vec![a.clone()]))
    );
    assert!(node.chart(v1.as_str()).is_some());
    node.deactivate(&a);
    assert_eq!(
        node.remove_version(v1.as_str()),
        Err(RemoveError::InUse(vec![a]))
    );
}

// @tg: verifies:: [[openspec:chart-versioning#Pin Until Final#Existing actor stays pinned]] {level: integration}
#[test]
fn existing_actor_stays_pinned() {
    let (mut node, _clock, _store) = fresh();
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    let a = va("pay", "a");
    node.send(&a, ev("pay")).unwrap();
    node.run_until_idle();
    node.register_type(&ty("pay"), scxml("Idle", V2_SAME))
        .unwrap();
    node.send(&a, ev("ok")).unwrap();
    node.run_until_idle();
    node.deactivate(&a);
    node.send(&a, ev("later")).unwrap();
    node.run_until_idle();
    assert_eq!(versions(&node, &a), vec![v1.as_str().to_owned(); 3]);
}

// @tg: verifies:: [[openspec:chart-versioning#Pin Until Final#New actor gets the latest]] {level: integration}
#[test]
fn new_actor_gets_the_latest() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    let v2 = node
        .register_type(&ty("pay"), scxml("Idle", V2_SAME))
        .unwrap();
    let b = va("pay", "brand-new");
    node.send(&b, ev("pay")).unwrap();
    node.run_until_idle();
    assert_eq!(versions(&node, &b), vec![v2.as_str().to_owned()]);
}

// @tg: verifies:: [[openspec:chart-versioning#Explicit Migration#Migration applied]] {level: integration}
#[test]
fn migration_applied() {
    let (mut node, _clock, _store) = fresh();
    let a = va("pay", "a");
    let (v1, v2) = with_migration(&mut node, &a);
    let pos = node.migrate(&a, "urn:vertep:migration:card2").unwrap();
    assert_eq!(pos, 2);
    let j = node.journal(&a).unwrap();
    assert_eq!(j.len(), 2);
    let EntryBody::Migration {
        migration,
        from,
        before,
        after,
    } = &j[1].body
    else {
        panic!("migration entry")
    };
    assert_eq!(migration, "urn:vertep:migration:card2");
    assert_eq!(
        (from.as_str(), before.version.as_str()),
        (v1.as_str(), v1.as_str())
    );
    assert_eq!(
        (j[1].version.as_str(), after.version.as_str()),
        (v2.as_str(), v2.as_str())
    );
    assert_eq!(
        node.state(&a).unwrap().configuration,
        vec!["Paying", "Paying.Card2"]
    );
    node.send(&a, ev("ok")).unwrap();
    node.run_until_idle();
    assert_eq!(versions(&node, &a)[2], v2, "later entries run under v2");
    assert_eq!(node.state(&a).unwrap().configuration, vec!["Done"]);
}

// @tg: verifies:: [[openspec:chart-versioning#Explicit Migration#Migration is atomic]] {level: integration}
#[test]
fn migration_is_atomic() {
    let (mut node, _clock, _store) = fresh();
    let a = va("pay", "a");
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    node.send(&a, ev("pay")).unwrap();
    node.run_until_idle();
    let v2 = node
        .register_type(&ty("pay"), scxml("Idle", V2_SAME))
        .unwrap();
    node.register_migration(
        Migration::identity("urn:vertep:migration:broken", v1.clone(), v2)
            .with_transform(|_| Err("cannot convert the cart".to_owned())),
    );
    let r = node.migrate(&a, "urn:vertep:migration:broken");
    assert!(matches!(r, Err(MigrateError::TransformFailed(_))));
    assert_eq!(node.journal(&a).unwrap().len(), 1, "no migration entry");
    node.send(&a, ev("ok")).unwrap();
    node.run_until_idle();
    assert_eq!(
        versions(&node, &a),
        vec![v1.as_str().to_owned(); 2],
        "continues under v1"
    );
}

// @tg: verifies:: [[openspec:chart-versioning#Explicit Migration#Migrations are in the graph]] {level: integration}
#[test]
fn migrations_are_in_the_graph() {
    let (mut node, _clock, store) = fresh();
    let a = va("pay", "a");
    let (v1, v2) = with_migration(&mut node, &a);
    node.migrate(&a, "urn:vertep:migration:card2").unwrap();
    let q = format!(
        "PREFIX vt: <https://w3id.org/vertep/vt#>
         SELECT ?from ?to ?m ?pos WHERE {{ GRAPH ?e {{ ?e a vt:MigrationEntry ; vt:actor <{a}> ;
           vt:fromVersion ?from ; vt:toVersion ?to ; vt:migration ?m ; vt:position ?pos }} }}"
    );
    let sol = store.query(&q).unwrap().into_solutions().unwrap();
    assert_eq!(sol.len(), 1);
    assert_eq!(sol.get(0, "from").unwrap().to_string(), format!("<{v1}>"));
    assert_eq!(sol.get(0, "to").unwrap().to_string(), format!("<{v2}>"));
    assert_eq!(
        sol.get(0, "m").unwrap().to_string(),
        "<urn:vertep:migration:card2>"
    );
    assert!(sol.get(0, "pos").unwrap().to_string().starts_with("\"2\""));
}

// @tg: verifies:: [[openspec:chart-versioning#Migration Compatibility Check#Unmapped configuration refused]] {level: integration}
#[test]
fn unmapped_configuration_refused() {
    let (mut node, _clock, _store) = fresh();
    let a = va("pay", "a");
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    node.send(&a, ev("pay")).unwrap();
    node.run_until_idle();
    let v2 = node
        .register_type(&ty("pay"), scxml("Idle", V2_RENAMED))
        .unwrap();
    node.register_migration(Migration::identity("urn:vertep:migration:naive", v1, v2));
    let err = node.migrate(&a, "urn:vertep:migration:naive").unwrap_err();
    let want: BTreeSet<String> = ["Paying", "Paying.Card"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(err, MigrateError::Unmapped(vec![want]));
    assert!(err.to_string().contains("{Paying, Paying.Card}"));
    assert_eq!(node.journal(&a).unwrap().len(), 1);
}

// @tg: verifies:: [[openspec:chart-versioning#Migration Compatibility Check#Identical structure needs no mapping]] {level: integration}
#[test]
fn identical_structure_needs_no_mapping() {
    let (mut node, _clock, _store) = fresh();
    let a = va("pay", "a");
    let v1 = node.register_type(&ty("pay"), scxml("Idle", V1)).unwrap();
    node.send(&a, ev("pay")).unwrap();
    node.run_until_idle();
    let v2 = node
        .register_type(&ty("pay"), scxml("Idle", V2_SAME))
        .unwrap();
    let from = node.chart(v1.as_str()).unwrap();
    let to = node.chart(v2.as_str()).unwrap();
    let m = Migration::identity("urn:vertep:migration:same", v1, v2);
    assert!(
        vertep_runtime::chart::unmapped_configurations(&m, &from, &to)
            .unwrap()
            .is_empty()
    );
    node.register_migration(m);
    node.migrate(&a, "urn:vertep:migration:same").unwrap();
    assert_eq!(
        node.state(&a).unwrap().configuration,
        vec!["Paying", "Paying.Card"]
    );
}

// @tg: verifies:: [[openspec:chart-versioning#Fork Before Migration#Rolling back a migration]] {level: integration}
#[test]
fn rolling_back_a_migration() {
    let (mut node, _clock, _store) = fresh();
    let a = va("pay", "a");
    let (v1, v2) = with_migration(&mut node, &a);
    let pos = node.migrate(&a, "urn:vertep:migration:card2").unwrap();
    let before = node.journal(&a).unwrap();
    let back = Address::parse("urn:vertep:test:rolled-back").unwrap();
    node.fork(&a, pos - 1, &back).unwrap();
    let s = node.state(&back).unwrap();
    assert_eq!(s.version, v1);
    assert_eq!(s.configuration, vec!["Paying", "Paying.Card"]);
    assert_eq!(
        node.journal(&a).unwrap(),
        before,
        "the original journal is unchanged"
    );
    assert_eq!(node.state(&a).unwrap().version, v2);
    node.send(&back, ev("ok")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&back).unwrap().configuration, vec!["Done"]);
    assert_eq!(node.journal(&back).unwrap()[1].version, v1);
}

// @tg: verifies:: [[openspec:chart-versioning#Self-Migration Requests#Chart requests an upgrade]] {level: integration}
#[test]
fn chart_requests_an_upgrade() {
    let (mut node, _clock, _store) = fresh();
    let v2_chart = scxml(
        "Idle",
        r##"<state id="Idle"><transition event="*"/></state><state id="Busy"/>"##,
    );
    let v2 = node.register_chart(v2_chart).unwrap();
    let v1 = node
        .register_type(
            &ty("self"),
            scxml(
                "Idle",
                &format!(
                    r##"<state id="Idle"><transition event="upgrade"><vt:migrate to="{v2}" migration="urn:vertep:migration:self"/></transition>
                    <transition event="*"/></state>"##,
                    v2 = v2.as_str()
                ),
            ),
        )
        .unwrap();
    node.register_migration(Migration::identity(
        "urn:vertep:migration:self",
        v1.clone(),
        v2.clone(),
    ));
    let a = va("self", "a");
    node.send(&a, ev("upgrade")).unwrap();
    node.send(&a, ev("next")).unwrap();
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    let kinds: Vec<&str> = j
        .iter()
        .map(|e| match &e.body {
            EntryBody::Input { .. } => "input",
            EntryBody::Migration { .. } => "migration",
            EntryBody::Fork { .. } => "fork",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["input", "migration", "input"],
        "applied after the requesting entry, before the next input"
    );
    assert_eq!(j[0].version, v1.as_str());
    assert_eq!(j[2].version, v2.as_str());
}

#[test]
fn migration_from_the_wrong_version_is_refused() {
    let (mut node, _clock, _store) = fresh();
    let a = va("pay", "a");
    let (_v1, v2) = with_migration(&mut node, &a);
    node.migrate(&a, "urn:vertep:migration:card2").unwrap();
    let r = node.migrate(&a, "urn:vertep:migration:card2");
    assert!(matches!(r, Err(MigrateError::WrongVersion { actual, .. }) if actual == v2));
}

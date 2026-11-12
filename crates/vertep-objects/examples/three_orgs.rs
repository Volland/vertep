//! A durable support agent kept by three organisations.
//!
//! The agent is a durable object: its address is a `did:webvh` DID, and its journal is kept by
//! three custodians, one per organisation, that talk only over TSP:
//!
//! - org 1 runs host A, which serves the agent first;
//! - org 2 runs host B, a standby;
//! - org 3 runs keeper K, which stores only encrypted entries and is reachable only through a
//!   TSP mediator (a store-and-forward intermediary) it picks up from.
//!
//! Host A starts the agent: a model call summarises the ticket, then the agent waits in a
//! decide state for a person to approve. Every input the agent accepts is journaled through
//! custody: A seals it, and it counts only once two of the three custodians accepted it. Then
//! org 1 disappears. Host B wins the next custody epoch from B and K, adopts the committed
//! journal, and replays the agent without calling the model again; the pending approval is
//! still open with the same correlation id, and the person answers it on host B.
//!
//! Run with `cargo run -p vertep-objects --example three_orgs --features testkit,tsp`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use vertep_agent::decider::{Decision, DecisionOutcome, HumanDecider, Inbox};
use vertep_agent::host::AgentHost;
use vertep_agent::llm::ScriptedProvider;
use vertep_agent::{AgentJournal, StoreAgentJournal};
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_federation::did::{Service, ServiceEndpoint};
use vertep_federation::testkit::TestIntermediary;
use vertep_federation::tsp::{TspError, TspIdentity, TspSession, Vid, vid_from_document};
use vertep_federation::{
    ActorIdentity, Did, DidMethodChoice, DidResolver, IdentityConfig, InMemoryKeyManager,
    KeyManager, LocalNetwork, RelayEndpoint, RelayNetwork, ResolverRegistry,
};
use vertep_objects::custody::tsp_link::{
    CustodyPort, ReplyInbox, ResolveVid, TspCustodyEndpoint, TspCustodyTransport, pump_mailbox,
};
use vertep_objects::custody::{
    Custodian, CustodyConfig, CustodyHost, CustodyStore, Replica, ResolverKeyDirectory,
};
use vertep_objects::{DurableObjectId, KeyManagerSigner};
use vertep_runtime::VirtualClock;
use vertep_store::{JournalStore, MemoryStore};

type Error = Box<dyn std::error::Error>;

const AGENT_CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
       version="1.0" datamodel="cel" initial="idle">
  <datamodel><data id="summary" expr="''"/></datamodel>
  <state id="idle"><transition event="task" target="summarising"/></state>
  <state id="summarising">
    <invoke type="vt:llm" id="summarise" vt:provider="model" vt:model="claude-opus-5-5"
            vt:template="Summarise the ticket in one line."/>
    <transition event="done.invoke.summarise" target="review">
      <assign location="summary" expr="ev.data.text"/>
    </transition>
  </state>
  <state id="review" vt:decide="human/support" vt:candidates="approve escalate">
    <transition event="approve" target="approved"/>
    <transition event="escalate" target="escalated"/>
  </state>
  <state id="approved"/><state id="escalated"/>
</scxml>"#;

/// One organisation's party: a `did:peer` with TSP keys.
struct Org {
    did: String,
    keys: Arc<InMemoryKeyManager>,
    identity: ActorIdentity,
    session: Arc<TspSession>,
}

fn org(uri: &str) -> Result<Org, Error> {
    let keys = Arc::new(InMemoryKeyManager::new());
    let identity = ActorIdentity::provision(
        keys.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![Service {
                id: "#tsp".into(),
                types: vec!["TSPTransport".into()],
                endpoints: vec![ServiceEndpoint::uri(uri)],
            }],
        }),
    )?;
    let me = TspIdentity::new(
        &identity,
        keys.clone() as Arc<dyn KeyManager>,
        url::Url::parse(uri)?,
    )
    .map_err(|e| e.to_string())?;
    Ok(Org {
        did: identity.did.clone(),
        keys,
        identity,
        session: Arc::new(TspSession::new(me, false)),
    })
}

fn signer(o: &Org) -> Arc<KeyManagerSigner> {
    Arc::new(KeyManagerSigner::new(
        &o.did,
        o.keys.clone() as Arc<dyn KeyManager>,
        o.identity.signing.key_name.clone(),
    ))
}

fn agent_host(store: Arc<CustodyStore>, model: Arc<ScriptedProvider>, inbox: Inbox) -> AgentHost {
    AgentHost::builder(Arc::new(VirtualClock::new(1_794_355_200_000)))
        .journal(Arc::new(StoreAgentJournal::new(store)))
        .provider("model", model)
        .decider("human/support", Arc::new(HumanDecider::new(inbox)))
        .build()
}

fn chart() -> Result<vertep_core::ir::Chart, Error> {
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    Ok(vertep_scxml::read(AGENT_CHART, &opts)
        .map_err(|e| format!("{e:?}"))?
        .chart)
}

fn main() -> Result<(), Error> {
    let net = LocalNetwork::new();
    let resolver: Arc<dyn DidResolver> =
        Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
    let resolve: ResolveVid = {
        let r = resolver.clone();
        Arc::new(move |id: &str| -> Result<Vid, TspError> {
            let did = Did::parse(id).map_err(|e| TspError::Open(e.0))?;
            let doc = r
                .resolve(&did)
                .map_err(|e| TspError::Open(e.to_string()))?
                .document;
            vid_from_document(&doc)
        })
    };

    // Org 3's mediator holds messages for the keeper until it picks them up.
    let mediator_uri = "https://mediator.org3.example/tsp";
    let mediator = TestIntermediary::new(
        Arc::new(InMemoryKeyManager::new()),
        mediator_uri,
        net.clone() as Arc<dyn RelayNetwork>,
        resolver.clone(),
        true,
    )?;
    net.serve(mediator_uri, mediator.clone() as Arc<dyn RelayEndpoint>);
    let mediator_vid = resolve(mediator.did())?;

    let a = org("https://host.org1.example/tsp")?;
    let b = org("https://host.org2.example/tsp")?;
    let k = org(mediator_uri)?;
    println!("org 1 host  {}", &a.did[..32]);
    println!("org 2 host  {}", &b.did[..32]);
    println!("org 3 keeper {} (behind its mediator)", &k.did[..32]);

    let agent = DurableObjectId::new("did:webvh:QmScid:support.example:agents:ticket-8812")?;
    let directory = Arc::new(ResolverKeyDirectory::new(resolver.clone()));
    let config = CustodyConfig::new(
        vec![
            Custodian::host(&a.did),
            Custodian::host(&b.did),
            Custodian::keeper(&k.did),
        ],
        2,
    )?;
    let replica = |o: &Org| -> Result<Arc<Replica>, Error> {
        Ok(Arc::new(Replica::open(
            agent.iri(),
            signer(o),
            Arc::new(MemoryStore::new()) as Arc<dyn JournalStore>,
            directory.clone(),
            config.clone(),
        )?))
    };
    let (ra, rb, rk) = (replica(&a)?, replica(&b)?, replica(&k)?);

    // Each host's TSP endpoint answers custody requests and receives its own answers.
    let mut inboxes = Vec::new();
    for (o, r, uri) in [
        (&a, &ra, "https://host.org1.example/tsp"),
        (&b, &rb, "https://host.org2.example/tsp"),
    ] {
        let inbox = ReplyInbox::new(o.session.clone(), resolve.clone());
        let endpoint = TspCustodyEndpoint::new(o.session.clone(), r.clone(), resolve.clone());
        net.serve(uri, Arc::new(CustodyPort::new(endpoint, inbox.clone())));
        inboxes.push(inbox);
    }
    // The keeper polls its mediator.
    let stop = Arc::new(AtomicBool::new(false));
    let pump = {
        let keeper = TspCustodyEndpoint::new(k.session.clone(), rk.clone(), resolve.clone());
        let (stop, mediator, net, kd) = (stop.clone(), mediator.clone(), net.clone(), k.did.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let _ = pump_mailbox(&keeper, mediator.as_ref(), &kd, net.as_ref());
                thread::sleep(Duration::from_millis(2));
            }
        })
    };

    let custody_host = |o: &Org, r: &Arc<Replica>, other: &Org, inbox: &Arc<ReplyInbox>| {
        CustodyHost::new(
            agent.clone(),
            signer(o),
            o.keys.clone() as Arc<dyn KeyManager>,
            o.identity.agreement.key_name.clone(),
            r.clone(),
            directory.clone(),
        )
        .with_transport(Arc::new(TspCustodyTransport::direct(
            &other.did,
            o.session.clone(),
            net.clone() as Arc<dyn RelayNetwork>,
            resolve.clone(),
        )))
        .with_transport(Arc::new(TspCustodyTransport::routed(
            &k.did,
            o.session.clone(),
            net.clone() as Arc<dyn RelayNetwork>,
            resolve.clone(),
            vec![mediator_vid.clone()],
            inbox.clone(),
            Duration::from_secs(5),
        )))
    };

    // ---- org 1 serves the agent ---------------------------------------------------------------
    let correlation = {
        let host_a = Arc::new(custody_host(&a, &ra, &b, &inboxes[0]));
        let store = Arc::new(CustodyStore::new(Arc::new(MemoryStore::new())));
        store.add(host_a.clone());
        let model = Arc::new(ScriptedProvider::new());
        model.push_text("Customer cannot log in after the password reset.", 40, 12);
        let people = Inbox::default();
        let mut agents = agent_host(store.clone(), model.clone(), people.clone());
        let v = agents.register(chart()?)?;
        agents.spawn(agent.as_str(), &v)?;
        agents.send(agent.as_str(), Event::named("task", EventType::External));
        agents.run_until_idle();
        let state = agents.state(agent.as_str())?;
        println!(
            "org 1: agent in {:?}, custody epoch {:?}, {} entries committed, model calls {}",
            state.configuration,
            host_a.writer_epoch(),
            host_a.committed().get(),
            model.calls()
        );
        println!(
            "       keeper K holds {} encrypted entries",
            rk.head().get()
        );
        people.pending()[0].correlation.clone()
    }; // org 1 is gone: its host, its process and its endpoint
    net.set_down("https://host.org1.example/tsp", true);

    // ---- org 2 takes over --------------------------------------------------------------------
    let host_b = Arc::new(custody_host(&b, &rb, &a, &inboxes[1]));
    let store = Arc::new(CustodyStore::new(Arc::new(MemoryStore::new())));
    store.add(host_b.clone());
    let model = Arc::new(ScriptedProvider::new()); // replay must never call it
    let people = Inbox::default();
    let mut agents = agent_host(store.clone(), model.clone(), people.clone());
    agents.register(chart()?)?;
    let state = agents.state(agent.as_str())?;
    println!(
        "org 2: won custody epoch {:?}, replayed {:?}, model calls {}",
        host_b.writer_epoch(),
        state.configuration,
        model.calls()
    );
    let offers = agents.offers(agent.as_str());
    println!(
        "       open approval {} (same as before the failover: {})",
        offers[0].correlation,
        offers[0].correlation == correlation
    );
    agents.run_until_idle();
    people.answer(
        &correlation,
        DecisionOutcome::Decision(Decision::new("approve", Value::Null)),
    );
    agents.run_until_idle();
    let state = agents.state(agent.as_str())?;
    let journal = StoreAgentJournal::new(store).read(agent.as_str());
    println!(
        "org 2: agent in {:?} after {} journaled inputs",
        state.configuration,
        journal.len()
    );
    stop.store(true, Ordering::SeqCst);
    let _ = pump.join();
    Ok(())
}

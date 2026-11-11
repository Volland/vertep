//! Two organisations' nodes exchange an event through an untrusted DIDComm v2 mediator.
//!
//! Org A's `buyer` and org B's `shop` are virtual actors whose addresses are `did:peer` DIDs.
//! The buyer's `<send>` to the shop's DID is resolved through the shop's DID document to the
//! mediator, authcrypted to the shop and wrapped in a forward; the shop's node picks it up,
//! verifies the sender, checks the grant, journals the event with `_event.origin` set to the
//! buyer's DID and only then acknowledges it to the mediator.
//!
//! Run with `cargo run -p vertep-federation --example two_orgs --features didcomm,testkit`.

use std::sync::Arc;
use std::time::Duration;

use vertep_core::datamodel::{Event, EventType, Value};
use vertep_federation::didcomm::DidCommProtocol;
use vertep_federation::testkit::TestMediator;
use vertep_federation::{
    DidMethodChoice, Federation, Grant, IdentityConfig, InMemoryKeyManager, KeyManager,
    LocalNetwork, PickupBinding, Recipients, ResolverRegistry, RuntimeClock, Service,
    ServiceEndpoint,
};
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{Address, Node, Shell, VirtualClock};
use vertep_store::MemoryStore;

const MEDIATOR: &str = "https://mediator.example/didcomm";

fn chart(body: &str) -> vertep_core::ir::Chart {
    let xml = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="s">{body}</scxml>"#
    );
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(&xml, &opts).expect("chart").chart
}

struct Org {
    node: Node,
    fed: Federation,
}

fn org(
    clock: &VirtualClock,
    net: &Arc<LocalNetwork>,
    reg: &Arc<ResolverRegistry>,
    m: &Arc<TestMediator>,
) -> Org {
    let store = Arc::new(MemoryStore::new());
    let fed = Federation::builder(reg.clone(), net.clone())
        .clock(Arc::new(RuntimeClock(Arc::new(clock.clone()))))
        .protocol(Arc::new(DidCommProtocol))
        .pickup(PickupBinding {
            name: "mediator".into(),
            source: m.clone(),
            protocol: Arc::new(DidCommProtocol),
            recipients: Recipients::Shared,
        })
        .storage(store.clone())
        .build();
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock.clone()),
    };
    let mut node = Node::builder(store, shell)
        .datamodels(Arc::new(vertep_dm_ecma::EcmaRegistry))
        .mailboxes_for_scheme("did", fed.mailboxes())
        .build();
    node.register_type(
        "https://example.org/charts/buyer",
        chart(
            r#"<state id="s"><transition event="buy">
                 <send event="order.placed" targetexpr="_event.data" id="o-1">
                   <content expr="({sku: 'tea', qty: 2})"/>
                 </send>
               </transition><transition event="*"/></state>"#,
        ),
    )
    .expect("buyer");
    node.register_type(
        "https://example.org/charts/shop",
        chart(r#"<state id="s"><transition event="*"/></state>"#),
    )
    .expect("shop");
    fed.attach(&node.handle());
    Org { node, fed }
}

fn main() {
    let clock = VirtualClock::new(1_794_355_200_000);
    let net = LocalNetwork::new();
    let reg = Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
    let mediator = TestMediator::new(
        Arc::new(InMemoryKeyManager::new()),
        MEDIATOR,
        net.clone(),
        reg.clone(),
    )
    .expect("mediator");
    net.serve(MEDIATOR, mediator.clone());
    let mut a = org(&clock, &net, &reg, &mediator);
    let mut b = org(&clock, &net, &reg, &mediator);

    let service = Service {
        id: "#didcomm".into(),
        types: vec!["DIDCommMessaging".into()],
        endpoints: vec![ServiceEndpoint {
            uri: MEDIATOR.into(),
            routing_keys: vec![mediator.routing_key()],
            accept: vec!["didcomm/v2".into()],
        }],
    };
    let config = IdentityConfig::new(DidMethodChoice::Peer {
        services: vec![service],
    });
    let km: Arc<dyn KeyManager> = Arc::new(InMemoryKeyManager::new());
    let buyer = a
        .fed
        .provision(
            &a.node.handle(),
            "https://example.org/charts/buyer",
            "alice",
            &config,
            km.clone(),
        )
        .expect("buyer DID");
    let shop = b
        .fed
        .provision(
            &b.node.handle(),
            "https://example.org/charts/shop",
            "tea",
            &config,
            km,
        )
        .expect("shop DID");
    a.fed.grant_outbound(Grant::new(
        buyer.did.as_str(),
        shop.did.as_str(),
        ["order.*"],
    ));
    b.fed.grant_inbound(Grant::new(
        buyer.did.as_str(),
        shop.did.as_str(),
        ["order.*"],
    ));

    let mut buy = Event::named("buy", EventType::External);
    buy.data = Value::String(shop.did.clone());
    a.node
        .send(&Address::parse(&buyer.did).expect("DID"), buy)
        .expect("send");
    for _ in 0..5 {
        a.node.run_until_idle();
        a.fed.pump();
        b.fed.pump();
        b.node.run_until_idle();
    }

    let shop_addr = Address::parse(&shop.did).expect("DID");
    for entry in b.node.journal(&shop_addr).expect("journal") {
        if let EntryBody::Input { input, .. } = &entry.body {
            println!(
                "shop journaled {} from {} via {} (send id {:?})",
                input.event.name,
                input.event.origin.as_deref().unwrap_or("?"),
                input.event.origintype.as_deref().unwrap_or("?"),
                input.event.sendid,
            );
        }
    }
    println!(
        "mediator still holds {} message(s)",
        mediator.store().held().len()
    );
}

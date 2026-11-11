//! The tiramemsu shard runs the whole shared backend test kit.

mod common;

use common::TempShard;

vertep_store::journal_store_kit!(TempShard::new);
vertep_store::graph_store_kit!(TempShard::new);
vertep_store::combined_store_kit!(TempShard::new);

#[test]
fn admitted_by_the_kit() {
    assert_eq!(
        vertep_store::testkit::admit_journal_store(TempShard::new),
        Ok(())
    );
    assert_eq!(
        vertep_store::testkit::admit_graph_store(TempShard::new),
        Ok(())
    );
}

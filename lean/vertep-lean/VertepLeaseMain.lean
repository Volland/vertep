import VertepLean.LeaseCheck

/-- `vertep-lease --history HISTORY.jsonl`. -/
def main (args : List String) : IO UInt32 := Vertep.Lease.Check.run args

import VertepLean.DecideCheck

/-- `vertep-decide --obs DECISIONS.jsonl`. -/
def main (args : List String) : IO UInt32 := Vertep.Agent.Check.run args

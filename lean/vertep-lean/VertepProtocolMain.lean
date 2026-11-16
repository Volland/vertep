import VertepLean.ProtocolCheck

/-- `vertep-kolyada --history HISTORY.jsonl [HISTORY.jsonl …]`. -/
def main (args : List String) : IO UInt32 := Vertep.Kolyada.Check.run args

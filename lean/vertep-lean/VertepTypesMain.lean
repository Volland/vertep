import VertepLean.Types.Oracle

/-- `vertep-types --cases CASES.jsonl`. -/
def main (args : List String) : IO UInt32 := Vertep.Types.Oracle.run args

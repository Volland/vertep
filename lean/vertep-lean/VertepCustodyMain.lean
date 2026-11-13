import VertepLean.CustodyJointCheck

/-- `vertep-custody --history HISTORY.jsonl`. -/
def main (args : List String) : IO UInt32 := Vertep.Custody.JointCheck.run args

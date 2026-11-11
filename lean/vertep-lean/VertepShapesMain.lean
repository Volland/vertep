import VertepLean.ShapesCheck

/-- `vertep-shapes FILE.nq…`. -/
def main (args : List String) : IO UInt32 := Vertep.ShapesCheck.run args

import VertepLean.Oracle

/-- `vertep-oracle --chart CHART.nq --records RECORDS.jsonl [--out TRACES.jsonl] [--compare]`. -/
def main (args : List String) : IO UInt32 := Vertep.Oracle.run args

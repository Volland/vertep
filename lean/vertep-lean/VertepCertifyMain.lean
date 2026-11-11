import VertepLean.Certify

/-- `vt-certify --chart CHART.nq [--out CERT.nt] [--time DATETIME]`. -/
def main (args : List String) : IO UInt32 := Vertep.Certify.run args

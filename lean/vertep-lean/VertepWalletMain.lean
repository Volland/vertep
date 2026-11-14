import VertepLean.WalletCheck

/-- `vertep-wallet --history HISTORY.jsonl [HISTORY.jsonl …]`. -/
def main (args : List String) : IO UInt32 := Vertep.Wallet.Check.run args

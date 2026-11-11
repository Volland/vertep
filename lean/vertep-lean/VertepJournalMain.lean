import VertepLean.JournalCheck

/-- `vertep-journal --obs OBS.json`. -/
def main (args : List String) : IO UInt32 := Vertep.Journal.run args

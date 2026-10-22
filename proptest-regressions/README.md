Counterexamples proptest ever finds live here, one file per source file,
kept so a bug fixed once can never silently come back. The directory is
part of the repository for the same reason the fixtures are: a regression
that only reproduces on runner CI, with no file to pin it, is a bug the
next lucky input can reintroduce.

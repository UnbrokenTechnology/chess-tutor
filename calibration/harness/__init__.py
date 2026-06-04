"""ELO-calibration harness (Component 3).

Local, offline tooling that turns bot dial-configs into measured Elo:
config generation -> fastchess gauntlet vs the Maia ladder -> Ordo
ratings -> dials->Elo model fit -> constrained solver.

Never shipped in the product (see ../README.md). Pure dev/measurement
code; the no-runtime-deps rule for the engine does not apply here.
"""

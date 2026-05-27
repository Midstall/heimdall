* Simple RC voltage divider with two output nodes for testing.
* The user-supplied "device netlist" includes a transient stimulus inside;
* heimdall wraps it with .tran + .save directives.

* RC divider: vin -> R1=1k -> mid -> R2=1k -> 0  (so V(mid) = V(vin)/2 in DC)
R1 in mid 1k
R2 mid 0 1k

* Cap to ground on the mid node; .ic forces V(mid)=0 at t=0 so the cap
* charging toward V(in)/2 produces a visible transient in coverage tests.
C1 mid 0 1n
.ic v(mid)=0

# Compatibility and wiring checks

This page previews the planned check panel and cited ratings on each part. It
does not work yet. The design is in
[`docs/design/compatibility-checks.md`](https://github.com/sjnorval/Ohmforge/blob/main/docs/design/compatibility-checks.md).
The batch command `ohmforge review` and the full limits table remain a P4 item
in
[`docs/design/ai-and-scenario-sweeps.md`](https://github.com/sjnorval/Ohmforge/blob/main/docs/design/ai-and-scenario-sweeps.md).

## Read a part's ratings

*Needs E6 and cited fields on the part.*

Select a part. The inspector lists supply range, logic levels and other ratings
that checks are allowed to use, each with a source (datasheet or standard, and
a page or clause). A figure that has not been reviewed is marked pending and
is not used as a hard fail.

## Run the checks

*Needs E4, E6, and the P3 first-slice rules.*

Open the Checks panel, or wait until P4 and run `ohmforge review`. Findings are
plain sentences, for example that an LED has no series resistor, or that an
output's high level does not meet a connected input's Vih at the stated supply.
Each finding names the parts and the citation, or says it is a topology rule
that needs no datasheet.

You apply any change yourself; Ohmforge does not rewrite the circuit.

## What the first slice covers

The P3 slice is small on purpose: LED without a series resistor, supply outside
a cited range, logic-level mismatch when both sides are cited, and missing
decoupling where a part cites a bypass requirement. I2C pull-ups, CAN
termination and the rest of the review rules follow in their existing phases.

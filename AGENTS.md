# Repository policy

- This file states policy only. Versions, paths, setup steps and current state belong to the sources, configs or generated docs that own them; cite or generate them, never retype them here.
- Change control runs in order: plan, spec, decision record, code. A superseded decision stays visible and is marked, never silently removed.
- A worker changes only files inside its own seam. A file under another writer's seam is handed over by message, never silently overwritten.
- Every claim in a change report carries the command and its exit code. A claim without a command is unverified; a check that never failed proves nothing.
- Browser behaviour is asserted from measured CDP results, never from narration. Report the command that produced the evidence.
- Keep browser control on the CDP path; do not introduce OS pointer, window or foreground-tab manipulation as an action or test path.
- Do not edit the vendored checker or its pin to make a red gate pass. A mismatch is a signal to investigate, not a reason to weaken the gate.
- Frozen artifacts and archives are not edit targets or migration sources.

## Approach
Remove LLM intent extraction, steering, and synthesis from ask. Return structured JSON of matched+linked units with also_available suggestions. Callers (Claude Code agents) do their own synthesis. Keep ask --synthesize as opt-in flag for human CLI use.

## Verify
simaris ask returns JSON in <200ms with no LLM calls. --synthesize flag still works for human use.

## Result
Fast, cheap, cache-friendly knowledge retrieval for agent workflows.

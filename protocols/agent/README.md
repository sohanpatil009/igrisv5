# protocols/agent — typed capability contract (v0.1)

AgentDef{name,capabilities[],allowed_tools[],tier}. Example:
{"agent":"coder","capabilities":["read_repository","modify_source","run_tests"]}.
External agents untrusted: validate identity, caps, payloads, artifacts.

#!/usr/bin/env bash
set -euo pipefail

PACKAGE="dev.agentremote.messenger"
RECEIVER="$PACKAGE/dev.agentremote.messenger.debug.NativeDebugCommandReceiver"
ACTION="$PACKAGE.DEBUG_COMMAND"
RESULT_FILE="files/agent-remote-native-result.json"
ADB_BIN="${ADB:-adb}"
COMMAND="${1:-status}"
shift || true

ID=""
PROJECT_ID=""
PROVIDER=""
TEXT=""
OUT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --id) ID="${2:?missing --id value}"; shift 2 ;;
    --project-id) PROJECT_ID="${2:?missing --project-id value}"; shift 2 ;;
    --provider) PROVIDER="${2:?missing --provider value}"; shift 2 ;;
    --text) TEXT="${2:?missing --text value}"; shift 2 ;;
    --out) OUT="${2:?missing --out value}"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

native() {
  local command="$1"
  shift
  local args=(shell am broadcast --receiver-foreground -a "$ACTION" -n "$RECEIVER" --es command "$command")
  while [[ $# -gt 0 ]]; do
    local key="$1"
    local value="$2"
    shift 2
    [[ -n "$value" ]] && args+=(--es "$key" "$value")
  done

  "$ADB_BIN" shell run-as "$PACKAGE" rm -f "$RESULT_FILE" >/dev/null 2>&1 || true

  local broadcast_output
  if ! broadcast_output=$("$ADB_BIN" "${args[@]}" 2>&1); then
    printf '%s\n' "$broadcast_output" >&2
    return 1
  fi

  local json_output
  if ! json_output=$("$ADB_BIN" shell run-as "$PACKAGE" cat "$RESULT_FILE" 2>/dev/null) || [[ -z "$json_output" ]]; then
    printf '%s\n' "$broadcast_output" >&2
    echo "native debug receiver did not produce a fresh JSON result" >&2
    return 1
  fi

  printf '%s\n' "$json_output"
  if command -v python3 >/dev/null 2>&1; then
    if ! python3 -c '
import json, sys
expected = sys.argv[1]
data = json.load(sys.stdin)
if data.get("ok") is not True:
    raise SystemExit(2)
if data.get("command") != expected:
    raise SystemExit(3)
' "$command" <<<"$json_output"; then
      return 2
    fi
  elif [[ "$json_output" != *'"ok":true'* || "$json_output" != *"\"command\":\"$command\""* ]]; then
    return 2
  fi
}

wait_native_ready() {
  local output=""
  local attempt
  for ((attempt = 1; attempt <= 25; attempt++)); do
    if output=$(native status 2>&1); then
      printf '%s\n' "$output"
      return 0
    fi
    if [[ "$output" != *"app_not_ready"* ]]; then
      printf '%s\n' "$output" >&2
      return 1
    fi
    sleep 0.2
  done
  echo "Agent Remote did not expose its native debug bridge after 25 attempts" >&2
  return 1
}

require_value() {
  local name="$1"
  local value="$2"
  [[ -n "$value" ]] || { echo "$name is required for $COMMAND" >&2; exit 2; }
}

case "$COMMAND" in
  help)
    cat <<'EOF'
Agent Remote Android AI-native debug commands
  bash ./ai-native.sh install
  bash ./ai-native.sh launch
  bash ./ai-native.sh status|dump|projects|conversations [--project-id UUID]
  bash ./ai-native.sh select-project --id UUID [--provider codex|grok]
  bash ./ai-native.sh select-conversation --id UUID
  bash ./ai-native.sh new|list
  bash ./ai-native.sh draft --text '...'
  bash ./ai-native.sh send [--text '...'] | steer [--text '...'] | interrupt
  bash ./ai-native.sh pair --text '<pair-url>' | connect-host --id UUID
  bash ./ai-native.sh smoke|logs|screenshot [--out path]|ui-dump [--out path]
EOF
    ;;
  install) "$(dirname "$0")/gradlew" :app:installDebug ;;
  launch) "$ADB_BIN" shell am start -W -n "$PACKAGE/.MainActivity" ;;
  status) native status ;;
  dump) native dump project_id "$PROJECT_ID" ;;
  projects) native projects ;;
  conversations) native conversations project_id "$PROJECT_ID" ;;
  select-project)
    require_value --id "$ID"
    native select_project id "$ID" provider "$PROVIDER"
    ;;
  select-conversation)
    require_value --id "$ID"
    native select_conversation id "$ID"
    ;;
  new) native new_conversation ;;
  list) native show_conversations ;;
  draft)
    require_value --text "$TEXT"
    native set_draft text "$TEXT"
    ;;
  send) native send text "$TEXT" ;;
  steer) native steer text "$TEXT" ;;
  interrupt) native interrupt ;;
  retry) native retry ;;
  disconnect) native disconnect ;;
  pair)
    require_value --text "$TEXT"
    native pair text "$TEXT"
    ;;
  connect-host)
    require_value --id "$ID"
    native connect_host id "$ID"
    ;;
  smoke)
    "$ADB_BIN" shell am start -W -n "$PACKAGE/.MainActivity"
    wait_native_ready
    native projects
    native conversations
    ;;
  logs) "$ADB_BIN" logcat -d -s AgentRemoteNative:I '*:S' ;;
  clear-logs) "$ADB_BIN" logcat -c ;;
  screenshot)
    OUT="${OUT:-$PWD/agent-remote-screen.png}"
    "$ADB_BIN" shell screencap -p /sdcard/Download/agent-remote-screen.png
    "$ADB_BIN" pull /sdcard/Download/agent-remote-screen.png "$OUT"
    echo "Saved $OUT"
    ;;
  ui-dump)
    OUT="${OUT:-$PWD/agent-remote-ui.xml}"
    "$ADB_BIN" shell uiautomator dump /sdcard/Download/agent-remote-ui.xml
    "$ADB_BIN" pull /sdcard/Download/agent-remote-ui.xml "$OUT"
    echo "Saved $OUT"
    ;;
  *) echo "unknown command: $COMMAND (run help)" >&2; exit 2 ;;
esac

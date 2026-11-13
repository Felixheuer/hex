#!/usr/bin/env bash
set -euo pipefail

# Writes synthetic "pause before a single final word" clips for the opt-in
# dictation corpus: a macOS `say` phrase, inserted silence long enough for the
# pause segmenter to cut, then one spoken word. Writes WAVs and
# synthetic/pause_manifest.json under AGENT_DESK_DICTATION_CORPUS_DIR. Never uses
# microphone audio.

CORPUS_DIR="${AGENT_DESK_DICTATION_CORPUS_DIR:-/private/var/folders/4t/_jnrck017r5f_jttbc19b7s40000gn/T/opencode/hex-dictation-corpus}"
PAUSE_MS="${AGENT_DESK_PAUSE_MS:-900}"
OUT="$CORPUS_DIR/synthetic"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"

# id|voice|rate|phrase|final word
CLIPS=(
  "pause_word_01_world_48k|Samantha|48000|So we built this little sandbox with our blocky guy, which is like an amazing|world"
  "pause_word_02_done_44k|Daniel|44100|When the checksum matches we load the model, and then we are finally|done"
  "pause_word_03_friday_48k|Karen|48000|Send the summary to the whole team and ask them to review it by|Friday"
  "pause_word_04_okay_44k|Moira|44100|Just show me the list with whatever controls you want, and that should be|okay"
)

entries=()
for clip in "${CLIPS[@]}"; do
  IFS='|' read -r id voice rate phrase word <<<"$clip"
  /usr/bin/say -v "$voice" -o "$WORK/$id-phrase.wav" --data-format="LEI16@$rate" "$phrase"
  /usr/bin/say -v "$voice" -o "$WORK/$id-word.wav" --data-format="LEI16@$rate" "$word"
  python3 - "$WORK/$id-phrase.wav" "$WORK/$id-word.wav" "$OUT/$id.wav" "$PAUSE_MS" <<'PY'
import sys, wave
phrase, word, out, pause_ms = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
frames = []
for path in (phrase, word):
    with wave.open(path) as source:
        params = source.getparams()
        frames.append(source.readframes(source.getnframes()))
silence = b"\0" * (params.framerate * pause_ms // 1000 * params.sampwidth * params.nchannels)
with wave.open(out, "wb") as target:
    target.setparams(params)
    target.writeframes(frames[0] + silence + frames[1])
PY
  entries+=("{\"id\":\"$id\",\"source\":\"macos_say:$voice\",\"category\":\"tail_pause_word\",\"text\":\"$phrase $word\",\"last_word\":\"$(tr '[:upper:]' '[:lower:]' <<<"$word")\",\"wav\":\"$id.wav\"}")
done

{
  echo "["
  for index in "${!entries[@]}"; do
    separator=","
    [[ $index -eq $((${#entries[@]} - 1)) ]] && separator=""
    echo "  ${entries[$index]}$separator"
  done
  echo "]"
} >"$OUT/pause_manifest.json"
echo "wrote ${#entries[@]} clips and $OUT/pause_manifest.json"

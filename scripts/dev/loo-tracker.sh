#!/usr/bin/env bash
# Leave-one-out: for each test game, train on the other two (from zero / from priors) and evaluate
# 4 variants on the test game. Truth = red-team view (red champions always visible to red).
cd C:/Users/Admin/Desktop/ligalarp
E=./rust/target/release/examples/eval_tracker.exe
export TRUTH=red
S="$TEMP/rc-loo"
rm -rf "$S"; mkdir -p "$S"
GAMES=(replay-4027208890 replay-unknown-2026-11-12T09-21-59 replay-unknown-2026-11-12T01-27-38)
EMPTY='{"version":1,"multipliers":{},"score":{"n":0,"logModel":0,"logBaseline":0,"hits":0},"recent":[]}'
summ() { grep '^SUMMARY' | sed 's/^SUMMARY //'; }
for test in "${GAMES[@]}"; do
  echo "$EMPTY" > "$S/zero-$test.json"
  cp model/priors-v1.json "$S/prior-$test.json"
  for g in "${GAMES[@]}"; do
    [ "$g" = "$test" ] && continue
    TRAIN=1 MODEL="$S/zero-$test.json" $E "$g" > /dev/null 2>&1
    TRAIN=1 MODEL="$S/prior-$test.json" $E "$g" > /dev/null 2>&1
  done
  echo "TEST $test none $($E "$test" 2>&1 | summ)"
  echo "TEST $test priors $(MODEL=model/priors-v1.json $E "$test" 2>&1 | summ)"
  echo "TEST $test zero+train $(MODEL="$S/zero-$test.json" $E "$test" 2>&1 | summ)"
  echo "TEST $test priors+train $(MODEL="$S/prior-$test.json" $E "$test" 2>&1 | summ)"
done

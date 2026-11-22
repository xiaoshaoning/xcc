#!/bin/bash
cd /d/Projects/codes/xcc
total=0; n=0
for f in tests/corpus/*.c; do
  out=$(./target/release/xcc.exe --opt-level=2 -c "$f" -o /tmp/sz.obj 2>/dev/null) || continue
  sz=$(stat -c %s /tmp/sz.obj 2>/dev/null || echo 0)
  total=$((total+sz)); n=$((n+1))
done
echo "objects=$n total=$total"

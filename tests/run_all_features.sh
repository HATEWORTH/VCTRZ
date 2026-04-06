#!/bin/bash
set -e
CLI="cargo run --release -p vectorize-cli --"
IMG1="tests/pigeon head side.png"
IMG2="tests/80s Logos.jpg"
OUT="tests/review"
PASS=0
FAIL=0
ERRORS=""

run_test() {
    local name="$1"
    shift
    echo -n "  $name ... "
    if output=$($CLI "$@" 2>&1); then
        size=$(wc -c < "$OUT/${name}.svg" 2>/dev/null || echo 0)
        echo "OK (${size} bytes)"
        PASS=$((PASS+1))
    else
        echo "FAIL"
        ERRORS="${ERRORS}\n  FAIL: $name\n    $output\n"
        FAIL=$((FAIL+1))
    fi
}

echo "=== Engine Tests ==="
for engine in vtracer hybrid; do
    for preset in logo illustration photo hifi sketch; do
        run_test "${engine}_${preset}" "$IMG1" -o "$OUT/${engine}_${preset}.svg" --engine $engine -p $preset
    done
done

echo ""
echo "=== B/W Fast-Path (color_count=2) ==="
run_test "bw_vtracer" "$IMG1" -o "$OUT/bw_vtracer.svg" --engine vtracer -c 2
run_test "bw_hybrid" "$IMG1" -o "$OUT/bw_hybrid.svg" --engine hybrid -c 2

echo ""
echo "=== Extreme Settings ==="
run_test "max_quality" "$IMG1" -o "$OUT/max_quality.svg" --engine hybrid -p hifi --color-detail 100 --path-precision 100 --curve-smoothness 50 --noise-filter 0 --gradient-layers 100
run_test "min_quality" "$IMG1" -o "$OUT/min_quality.svg" --engine hybrid -p logo --color-detail 0 --path-precision 0 --curve-smoothness 100 --noise-filter 100 --gradient-layers 0
run_test "all_zero" "$IMG1" -o "$OUT/all_zero.svg" --engine hybrid --color-detail 0 --path-precision 0 --curve-smoothness 0 --noise-filter 0 --gradient-layers 0

echo ""
echo "=== Feature Flags ==="
run_test "shapes_on" "$IMG1" -o "$OUT/shapes_on.svg" --engine hybrid --detect-shapes
run_test "shapes_off" "$IMG1" -o "$OUT/shapes_off.svg" --engine hybrid
run_test "cutout_mode" "$IMG1" -o "$OUT/cutout_mode.svg" --engine hybrid --layer-mode cutout

echo ""
echo "=== Second Image (JPEG) ==="
run_test "logos_hybrid" "$IMG2" -o "$OUT/logos_hybrid_test.svg" --engine hybrid -p logo
run_test "logos_vtracer" "$IMG2" -o "$OUT/logos_vtracer_test.svg" --engine vtracer -p illustration
run_test "logos_hifi" "$IMG2" -o "$OUT/logos_hifi_test.svg" --engine hybrid -p hifi --detect-shapes

echo ""
echo "=== Fractional Settings ==="
run_test "frac_smooth" "$IMG1" -o "$OUT/frac_smooth.svg" --engine hybrid --curve-smoothness 33.7 --noise-filter 2.5
run_test "frac_detail" "$IMG1" -o "$OUT/frac_detail.svg" --engine hybrid --color-detail 87.3 --gradient-layers 66.6

echo ""
echo "==============================="
echo "PASS: $PASS  FAIL: $FAIL"
if [ $FAIL -gt 0 ]; then
    echo -e "\nFailures:$ERRORS"
fi

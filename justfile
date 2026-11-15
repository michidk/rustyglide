default:
    @just --list

# formatting, lints, licence and boundary checks, tests, GPU tests
gates:
    bash dev/scripts/gates.sh

lints:
    bash dev/scripts/gates.sh lints

test:
    bash dev/scripts/gates.sh test

# GPU tests on lavapipe (Docker when the host has no Vulkan)
gpu:
    bash dev/scripts/gates.sh gpu

# rewrite the golden images from the current renderer; review the diff before committing
bless:
    RUSTYGLIDE_BLESS=1 bash dev/scripts/gpu-test.sh golden

# all golden images side by side, for review
sheet out="contact_sheet.png":
    cargo run -q -p rustyglide-scenes --example contact_sheet -- {{out}}

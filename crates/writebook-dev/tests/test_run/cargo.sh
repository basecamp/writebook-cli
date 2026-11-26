#!/bin/sh
printf '%s\n' "$@" > "$LOG/cargo.args"
exit "$(cat "$LOG/cargo.status")"

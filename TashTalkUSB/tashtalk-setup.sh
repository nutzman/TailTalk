#!/bin/bash
set -e

configure_device() {
    output=$(cp210x-cfg -l)
    echo "$output"

    bus=$(echo "$output" | grep -oP '(?<=bus )\d+')
    dev=$(echo "$output" | grep -oP '(?<=dev )\d+')

    if [[ -z "$bus" || -z "$dev" ]]; then
        echo "Error: could not parse bus/dev from output" >&2
        return 1
    fi

    # Strip leading zeros for the device path
    bus=$((10#$bus))
    dev=$((10#$dev))

    echo "Found device at bus $bus, dev $dev"
    cp210x-cfg -d "$bus.$dev" -L 1 -N "TashTalk USB" -C "Feral Firmware"
    echo "Done. Waiting for next device..."
}

echo "Waiting for TashTalk USB to be plugged in... (Ctrl+C to quit)"

udevadm monitor --udev --subsystem-match=usb 2>/dev/null | while read -r line; do
    if echo "$line" | grep -q "add"; then
        # Extract the sysfs path from the event line and check for the Silicon Labs vendor ID
        syspath=$(echo "$line" | grep -oP '/devices/\S+')
        if [[ -n "$syspath" ]]; then
            vendor=$(cat "/sys$syspath/idVendor" 2>/dev/null)
            if [[ "$vendor" != "10c4" ]]; then
                continue
            fi
        fi
        # Give the device a moment to fully enumerate
        sleep 1
        configure_device || true
    fi
done

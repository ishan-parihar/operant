"""Operant Uno Q bridge app entrypoint.

Minimal placeholder bridge: opens the serial port, forwards commands from
stdin to the sketch, and prints responses. Replace with the full Operant
bridge implementation.
"""

import argparse
import sys

import serial  # pyserial


def main() -> int:
    parser = argparse.ArgumentParser(description="Operant Uno Q bridge")
    parser.add_argument("--port", required=True, help="Serial port (e.g. /dev/ttyACM0)")
    parser.add_argument("--baud", type=int, default=115200)
    args = parser.parse_args()

    try:
        ser = serial.Serial(args.port, args.baud, timeout=2)
    except serial.SerialException as exc:
        print(f"ERR: cannot open port: {exc}", file=sys.stderr)
        return 1

    print("OPERANT_BRIDGE_READY", flush=True)
    for line in sys.stdin:
        ser.write(line.encode())
        resp = ser.readline().decode(errors="replace").strip()
        if resp:
            print(resp, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

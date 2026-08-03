/*
 * Operant Arduino Uno base firmware.
 *
 * Implements a minimal line-based serial protocol:
 *   capabilities  -> lists supported commands
 *   gpio_read <pin>        -> reads digital pin, prints value
 *   gpio_write <pin> <0|1> -> writes digital pin
 *
 * Commands are newline-terminated; replies end with "OK" or "ERR:<msg>".
 */
const unsigned long SERIAL_TIMEOUT_MS = 1000;

void handle_command(String cmd) {
  cmd.trim();
  if (cmd == "capabilities") {
    Serial.println("capabilities,gpio_read,gpio_write");
    Serial.println("OK");
    return;
  }
  if (cmd.startsWith("gpio_read")) {
    int pin = cmd.substring(10).toInt();
    if (pin < 2 || pin > 13) {
      Serial.println("ERR:pin-out-of-range");
      return;
    }
    pinMode(pin, INPUT_PULLUP);
    Serial.print("value=");
    Serial.println(digitalRead(pin));
    Serial.println("OK");
    return;
  }
  if (cmd.startsWith("gpio_write")) {
    int space = cmd.indexOf(' ', 11);
    if (space < 0) {
      Serial.println("ERR:malformed-gpio_write");
      return;
    }
    int pin = cmd.substring(11, space).toInt();
    int value = cmd.substring(space + 1).toInt();
    if (pin < 2 || pin > 13 || (value != 0 && value != 1)) {
      Serial.println("ERR:invalid-args");
      return;
    }
    pinMode(pin, OUTPUT);
    digitalWrite(pin, value);
    Serial.println("OK");
    return;
  }
  Serial.println("ERR:unknown-command");
}

void setup() {
  Serial.begin(115200);
  while (!Serial) { delay(10); }
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, HIGH);
  Serial.println("OPERANT_READY");
}

void loop() {
  if (Serial.available() > 0) {
    String cmd = Serial.readStringUntil('\n');
    handle_command(cmd);
  }
  digitalWrite(LED_BUILTIN, HIGH);
  delay(SERIAL_TIMEOUT_MS);
  digitalWrite(LED_BUILTIN, LOW);
  delay(SERIAL_TIMEOUT_MS);
}

/*
 * Operant Uno Q bridge sketch.
 *
 * Minimal placeholder sketch: mirrors serial input to the builtin LED and
 * echoes a heartbeat. Replace with the full Operant bridge implementation.
 */
void setup() {
  Serial.begin(115200);
  pinMode(LED_BUILTIN, OUTPUT);
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  Serial.println("OPERANT_BRIDGE_READY");
  delay(1000);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1000);
}

/*
 * An interface to the Tracer solar regulator.
 * Communicating in a way similar to the MT-5 display
 */
#include <SoftwareSerial.h>
#include <Arduino.h>
#define BAUD 9600
#define DATA_DELAY (uint16_t)((3.5 / BAUD) * 1e6)
#define RX_PIN  10
#define TX_PIN  11
//Data delay of 3.5 char time as per modbus standards
// char time is (1.0 / baud rate)

unsigned int speed = 1000;          // Default update speed.
SoftwareSerial mppt_serial(RX_PIN, TX_PIN); // RX, TX

// DATA SYNCHRONIZATION BYTES
uint8_t start[12] = {0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55,
                     0xEB, 0x90, 0xEB, 0x90, 0xEB, 0x90};
uint8_t input[4];
char sep = ':';
String outString = "";
uint8_t led_state = 0;

const byte buff_size = 8;
char recv[buff_size];
boolean newInput = false;

double randomDouble(double minf, double maxf)
{
  return minf + random(1UL << 31) * (maxf - minf) / (1UL << 31);
}

void setup()
{
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(57600);
  while (!Serial)
  {
    ; // wait for serial port to connect. Needed for native USB port only
  }
  mppt_serial.begin(9600);
}

// Tested works OK
uint16_t crc(uint8_t *CRC_Buff, uint8_t crc_len)
{
  uint8_t crc_i, crc_j, r1, r2, r3, r4;
  uint16_t crc_result;
  r1 = *CRC_Buff;
  CRC_Buff++;
  r2 = *CRC_Buff;
  CRC_Buff++;
  for (crc_i = 0; crc_i < crc_len - 2; crc_i++)
  {
    r3 = *CRC_Buff;
    CRC_Buff++;
    for (crc_j = 0; crc_j < 8; crc_j++)
    {
      r4 = r1;
      r1 = (r1 << 1);
      if ((r2 & 0x80) != 0)
      {
        r1++;
      }
      r2 = r2 << 1;
      if ((r3 & 0x80) != 0)
      {
        r2++;
      }
      r3 = r3 << 1;
      if ((r4 & 0x80) != 0)
      {
        r1 = r1 ^ 0x10;
        r2 = r2 ^ 0x41;
      }
    }
  }
  crc_result = r1;
  crc_result = crc_result << 8 | r2;
  return crc_result;
}

// Convert two bytes to a float. OK
float to_float(uint8_t *buffer, int offset)
{
  unsigned short full = buffer[offset + 1] << 8 | buffer[offset];
  return full / 100.0;
}

void manualControlCmd(bool load_onoff)
{
  mppt_serial.write(start, sizeof(start));
  uint8_t mcc_data[] = {0x16, //DEVICE ID BYTE
                        0xAA, //COMMAND BYTE
                        0x01, //DATA LENGTH
                        0x00,
                        0x00, 0x00, //CRC CODE
                        0x7F};      //END BYTE
  if (load_onoff)
  {
    mcc_data[3] = 1;
  }
  else
  {
    mcc_data[3] = 0;
  }
  //Calculate and add CRC bytes.
  uint16_t crc_d = crc(mcc_data, mcc_data[2] + 5);
  mcc_data[mcc_data[2] + 3] = crc_d >> 8;
  mcc_data[mcc_data[2] + 4] = crc_d & 0xFF;
  mppt_serial.write(mcc_data, sizeof(mcc_data));
}

void printAllData()
{
  uint8_t data[] = {0x16,       //DEVICE ID BYTE
                    0xA0,       //COMMAND BYTE
                    0x00,       //DATA LENGTH
                    0xB1, 0xA7, //CRC CODE
                    0x7F};      //END BYTE
  
  uint16_t data_len = 256;
  uint8_t buff[data_len];
  mppt_serial.write(start, sizeof(start));
  mppt_serial.write(data, sizeof(data));

  int read = 0;

  for (int i = 0; i < data_len; i++)
  {
    if (mppt_serial.available())
    {
      buff[read] = mppt_serial.read();
      delayMicroseconds(DATA_DELAY);
      read++;
    }
  }
  #if defined(TESTING)
    float battery = randomDouble(9, 16);
    float pv = randomDouble(0, 11);
    float load_current = randomDouble(0, 15);
    float over_discharge = randomDouble(0, 17);
    float battery_max = randomDouble(0, 19);
    uint8_t full = randomDouble(0.0, 2.0); 
    uint8_t charging = randomDouble(0.0, 2.0); 
    int8_t battery_temp = randomDouble(-11.0, 30.0);
    float charge_current = randomDouble(0.0, 30.0);  
    uint8_t load_onoff = led_state;
  #else
    float battery = to_float(buff, 9);
    float pv = to_float(buff, 11);
    float load_current = to_float(buff, 15);
    float over_discharge = to_float(buff, 17);
    float battery_max = to_float(buff, 19);
    uint8_t full = buff[27]; 
    uint8_t charging = buff[28];
    int8_t battery_temp = buff[29] - 30;
    float charge_current = to_float(buff, 30);  
    uint8_t load_onoff = buff[21];
  #endif
  outString = 
      String(battery) + sep + String(pv) + sep + String(load_current) + sep + String(over_discharge) + sep + String(battery_max)
      + sep + String(full) + sep + String(charging) + sep + String(battery_temp) + sep + String(charge_current) + sep + String(load_onoff);
  
  Serial.print(outString);
}

void loop()
{
  printAllData();
  Serial.println();
  recvInput();
  if (newInput == true) {
    String inputStr = String(recv);
    if (inputStr == "LON") {
      digitalWrite(LED_BUILTIN, HIGH);
      manualControlCmd(true);
      led_state = 1;
    }
    if (inputStr == "LOFF") {
      digitalWrite(LED_BUILTIN, LOW);
      manualControlCmd(false);
      led_state = 0;
    }
    newInput = false;
  }
  delay(speed);
}

void recvInput() {
    static byte i = 0;
    char c;
    
    while (Serial.available() > 0 && newInput == false) {
        c = Serial.read();
        if (c != '\n') {
            recv[i] = c;
            i++;
            if (i >= buff_size) {
                i = buff_size - 1;
            }
        }
        else {
            recv[i] = '\0';
            i = 0;
            newInput = true;
        }
    }
}
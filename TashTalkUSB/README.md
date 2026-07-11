## Overview

[![I sell on Tindie](https://static.tindie.com/badges/tindie-mediums.png)](https://www.tindie.com/stores/feralfirmware/?ref=offsite_badges&utm_source=sellers_FeralFirmware&utm_medium=badges&utm_campaign=badge_medium)

This device us a USB-ified version of [TashTalk by Tashtari](https://github.com/lampmerchant/tashtalk) using the original PIC12F1840 and a CP2102N USB-to-UART chip.

It uses the same firmware (v2.1.3 as of time of writing) from the original project, an an SN65HVD1473 transceiver from TI so it can be
used with a regular LocalTalk cable, or a dongle like PhoneNET. 

### Programming PIC

These boards use a [TagConnect cable](https://www.tag-connect.com/product/tc2030-pkt-nl-6-pin-no-legs-cable-for-microchip-pickit-3) for connecting to the board for programming. If using the PICkit to power the circuit during bringup ensure that the supply voltage is set to 3.3V - 5V will destroy the 
other chips on the board.

I use [Microchip's IPE](https://www.microchip.com/en-us/tools-resources/production/mplab-integrated-programming-environment) for bulk programming these
devices. No extra settings need to be changed beyond the supply voltage if powering it off the PICkit. Just burn the firmware on to the device
and it should be ready to go. 

### Programming CP2102N

The CP2102N should work out of the box, but does not have the LED alternate mode immediately enabled (so the little RX/TX LEDs light up). I use [cp210x-cfg](https://github.com/cr1tbit/cp210x-cfg) along with [tashtalk-setup.sh](/TashTalkUSB/tashtalk-setup.sh) to provision these. This script only works on Linux
today but could be adapted for other platforms if needed.

This little script enables the LED mode and sets the device name and manufacturer so it comes up with a friendly name in TailTalk. It is not strictly necessary to do this of course.

The script launches and will wait for the first TashTalk USB device to be plugged in. Once detected it will then set all the values and wait
for the next one to be plugged in. It is pretty helpful for mass programming these for a bulk order. 

### Verification

Once both the PIC and CP2102N have been programmed I typically test this by launching TailTalk, starting the server and verifying a vintage Mac can
connect and see files from it. TashTalk can auto reconnect to TashTalk so if bulk testing you can simply unplug the USB and DIN cable, swap to another
and then resume testing.

The tests pass if the remote Mac can successfully discover the server and query files from it, and the LEDs work on the board. 

### Manufacturing 

I use JLCPCB for making these boards and the schematics and board files are already prepared with LCSC part numbers for each component. I've tried to
choose their basic components as much as possible to keep costs down. If doing a large order panelisation is recommended as the individual boards are 
really small and a high cost per board is paid when not panelised.
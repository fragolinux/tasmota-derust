# Tasmota SetOption defaults: firmware vs documentation

November 15, 2026. Findings made while building the SetOption editor of [Tasmota Derust](../README.md), written for the Tasmota developers.

The online Commands page gives the wrong default for 4 SetOptions compared with what Tasmota 16.0.0 sets on a freshly flashed device, describes 2 with outdated or reversed values, and leaves 2 out. More important: the development branch drops every upgrade step older than 10.0.0.3, not only those below 9.1, so a device on 9.1.0 (the last stop of the documented upgrade path) will skip a dozen migrations, SetOption57 among them.

## Wrong defaults

Four documented defaults are 1 where 16.0.0 leaves the setting at 0 on a new device. A real ESP8266 on 16.0.0 reports 0 for all four in `Status 3`.

| SetOption | Commands.md says | 16.0.0 on a new device | Evidence in the source |
| --- | --- | --- | --- |
| SO40 (button ignore time) | `(default =1)` | 0 | `P_HOLD_IGNORE` is not set in `SettingsDefaultSet2()`; the enum comment in `tasmota.h` itself says "default 0" |
| SO69 | `1 = enable Tuya dimmer 10% lower limit (default)` | 0 | the bit is now `sb_receive_invert` (14.2.0.3), not set in `SettingsDefaultSet2()`; `SettingsDelta()` clears it when upgrading from < 14.2.0.3 |
| SO93 (rule cache) | `1 = Keep uncompressed rules in memory … (default)` | 0 | `compress_rules_cpu` is not set in `SettingsDefaultSet2()` |
| SO128 (referer check) | `1 = enabled (default)` | 0 | `flag5.disable_referer_chk` is set only under `#ifdef DISABLE_REFERER_CHK`, which is commented out in `my_user_config.h` |

The other documented defaults match 16.0.0, including the numeric ones (SO32 40, SO33 5, SO34 200, SO36 1, SO41 60, SO42 90, SO44 25, SO45 40). SO115 depends on the ESP32 BLE build and was not checked.

## Outdated or reversed descriptions

Two entries describe values that no longer match the firmware, and some firmware comments (copied into `decode-status.py`) disagree with the documentation.

| SetOption | What the docs or comments say | What the firmware does |
| --- | --- | --- |
| SO128 | Commands.md: "Web referrer check for HTTP API commands — 0 = disabled, 1 = enabled (default)" | The bit is `disable_referer_chk`: 0 = referer check on (default), 1 = HTTP API accepted without the check. The values are reversed in the docs. `tasmota_types.h` and `decode-status.py` ("Allow access without referer check") are right. |
| SO69 | Commands.md: "Invert Serial receive on SerialBridge" as the heading, then "In older versions: Deprecated in favor of DimmerRange" and the old Tuya text with `1 = … (default)` | Since 14.2.0.3 only the SerialBridge meaning exists: 0 = normal receive (default), 1 = inverted. The Tuya lines and their default belong to versions before 14.2.0.3. |
| SO19 | `tasmota_types.h` and `decode-status.py`: "(HAss) Control automatic discovery (1)" | Commands.md is the current one: 0 = Tasmota discovery (default), 1 = the deprecated Home Assistant MQTT discovery, only with `USE_HOME_ASSISTANT`. The firmware comment still describes the old option. |
| SO95, SO96 | `decode-status.py`: "(Wifi) Enable Wifi", "(Eth) Enable Ethernet" listed as SetOptions | In `tasmota_types.h` these bits are `network_wifi` / `network_ethernet`, owned by the `Wifi` and `Ethernet` commands (`CMND_WIFI`, `CMND_ETHERNET`); Commands.md does not list them as SetOptions. |
| SO102 | `decode-status.py`: "(Teleinfo) Set Baud rate … 1200 (0) or 9600 (1)" | The bit is `obsolete1`: "Teleinfo config has now a dedicated bit field". |

## Defaults changed over time, and upgrade steps the development branch drops

Up to 16.0.0, `SettingsDelta()` brings upgraded devices in line with newer defaults. The development branch (16.0.0.5) removed every step older than 10.0.0.3, although its changelog only says "Support for migration from versions below v9.1" was removed. The official upgrade path in Upgrading.md ends with **9.1 → current release**, so a device that follows it to the next release will skip the 9.2–9.5 steps below.

### SetOptions changed by upgrade steps (16.0.0)

| SetOption | Set to | When upgrading from | In development 16.0.0.5 |
| --- | --- | --- | --- |
| SO57 (Wi-Fi rescan every 44 min) | 1 | < 9.2.0.3 (#10395) | removed: a 9.1 device keeps 0, a new device gets 1 |
| SO103 (MQTT TLS) | 1, only when the MQTT port is 8883, 8884 or 443 | < 9.5.0.5 | removed |
| SO128 (referer check) | 0, or 1 with `DISABLE_REFERER_CHK` | < 9.5.0.7 | removed |
| SO73 (detach buttons) | 0 | < 8.2.0.4 | removed (below 9.1, as announced) |
| SO41 (gratuitous ARP) | 60, when it was 0 | < 10.0.0.3 | kept |
| SO132 (TLS fingerprint) | 1, when a fingerprint is stored | < 10.0.0.3 | kept |
| SO45 (bistable pulse) | 40 | < 12.0.2.4 | kept |
| SO35 (serial skip) | 0 | < 12.2.0.5 | kept |
| SO69 (SerialBridge invert) | 0 | < 14.2.0.3 | kept |
| SO133, SO81 (Dingtian board only) | 0; SO81 = 1 with `DINGTIAN_INPUTS_INVERTED` | < 15.2.0.4 | kept |

SO57 history, from `my_user_config.h` at each release: `WIFI_SCAN_REGULARLY` is `false` up to 9.1.0 and `true` from 9.2.0.3 on. A real ESP8266 upgraded 6.7.1 → 7.2.0 → 8.5.1 → 9.1.0 → 16.0.0 reports SO57 = 1, set by the 9.2.0.3 step that the development branch no longer has.

### Other steps between 9.1 and 10.0.0.3 that are gone

The same commits ([05dbdf0](https://github.com/arendst/Tasmota/commit/05dbdf0483523067f538f6fe9846d073396a812e), [179264f](https://github.com/arendst/Tasmota/commit/179264f48070b4c81be91698612193858a791499), [099f97d](https://github.com/arendst/Tasmota/commit/099f97d4c9979a9eec41a0bc87ffa31ec174f992)) also drop these steps, which a device on 9.1.0 still needs:

- < 9.2.0.6: `switchmode[]` reset to `SWITCH_MODE`, `interlock[]` rebuilt from the old field
- < 9.2.0.7: `device_group_tie` initialised
- < 9.3.1.2: `mqtt_keepalive` and `mqtt_socket_timeout` initialised
- < 9.3.1.4 and < 9.4.0.2: `mbflag2` and `sbflag1` cleared
- < 9.5.0.2 (ESP32-C3 only): settings layout moved with `memmove`
- < 9.5.0.3: all sensors enabled (`Settings->sensors`)
- < 9.5.0.4: `power_lock` and the second DNS address
- < 9.5.0.5: range extender settings
- < 9.5.0.9: per-phase energy counters migrated

`mqtt_wifi_timeout` (< 9.4.0.6) is safe: `SettingsMinimum()` also sets it when it is 0. Keeping the 9.1.0.0–10.0.0.2 steps, or adding a 9.1 → 16.0.0 step to the documented upgrade path, would avoid all of this.

## Zero as built-in default

For two numeric options a stored 0 means "use the built-in value": the driver replaces it when it starts. The docs could say so, because a device without that driver keeps reporting 0 and looks changed from the documented default.

| SetOption | Commands.md | New device | Upgraded or driver-less device | Where 0 is replaced |
| --- | --- | --- | --- | --- |
| SO39 (CSE7766 invalid power) | default 128, "0 = reset to default on next restart" | 0 (not set in `SettingsDefaultSet2()`) | 0 | `xnrg_02_cse7766.ino`: 0 → `CSE_MAX_INVALID_POWER` |
| SO44 (IR tolerance) | default 25, range 1..100 | 25 | 0 when upgraded from a version without it | `xdrv_05_irremote*.ino`: 0 → `IR_RCV_TOLERANCE` |

The ESP8266 above (16.0.0, upgraded from 6.7.1) reports 0 for both.

## Missing or incomplete entries in Commands.md

Commands.md covers 147 of the 150 SetOption slots that 16.0.0 defines (SO49 is a free slot). The gaps:

- **SO106** (Virtual CT for RGBW lights, `LIGHT_VIRTUAL_CT`): no entry.
- **SO102**: no entry; the bit is obsolete, which could be stated so tools do not label it as the Teleinfo baud rate.
- **SO38** (IR unknown-message threshold): the range `6..255` is given but not the default, which is 6 (`IR_RCV_MIN_UNKNOWN_SIZE`).
- **SO166** (device location in `tele/STATE`) is documented but exists only in the development branch, not in 16.0.0.

16 SetOptions are documented only in the row of another command (for example `BuzzerActive<BR>SetOption67`, `ZbDeviceTopic<BR>SetOption89`), not in a `SetOptionN` row of their own. That is fine for people, but tools that read the table have to know the pattern.

## Sources

Defaults were read from the source of each release; the device values from a Wemos D1 mini (ESP8266) on 16.0.0 that went through the documented upgrade path 6.7.1 → 7.2.0 → 8.5.1 → 9.1.0 → 16.0.0, with `Status 3` and the `SetOptionN` commands.

- Tasmota 16.0.0: [settings.ino](https://github.com/arendst/Tasmota/blob/v16.0.0/tasmota/tasmota_support/settings.ino) (`SettingsDefaultSet2()`, `SettingsDelta()`), [my\_user\_config.h](https://github.com/arendst/Tasmota/blob/v16.0.0/tasmota/my_user_config.h), [tasmota\_types.h](https://github.com/arendst/Tasmota/blob/v16.0.0/tasmota/include/tasmota_types.h), [tasmota.h](https://github.com/arendst/Tasmota/blob/v16.0.0/tasmota/include/tasmota.h), [tasmota\_globals.h](https://github.com/arendst/Tasmota/blob/v16.0.0/tasmota/include/tasmota_globals.h), [decode-status.py](https://github.com/arendst/Tasmota/blob/v16.0.0/tools/decode-status.py)
- Development branch, 16.0.0.5 at [99907bd](https://github.com/arendst/Tasmota/commit/99907bd8f5c04d8af4ba8491c801fb316276419b): [settings.ino](https://github.com/arendst/Tasmota/blob/99907bd8f5c04d8af4ba8491c801fb316276419b/tasmota/tasmota_support/settings.ino)
- SO57 history: `my_user_config.h` at [v8.5.1](https://github.com/arendst/Tasmota/blob/v8.5.1/tasmota/my_user_config.h), [v9.1.0](https://github.com/arendst/Tasmota/blob/v9.1.0/tasmota/my_user_config.h), [v10.0.0](https://github.com/arendst/Tasmota/blob/v10.0.0/tasmota/my_user_config.h); [CHANGELOG at v10.0.0](https://github.com/arendst/Tasmota/blob/v10.0.0/CHANGELOG.md) (9.2.0.3: "Force initial default state SetOption57 1", #10395)
- Drivers: [xnrg\_02\_cse7766.ino](https://github.com/arendst/Tasmota/blob/v16.0.0/tasmota/tasmota_xnrg_energy/xnrg_02_cse7766.ino), [xdrv\_05\_irremote.ino](https://github.com/arendst/Tasmota/blob/v16.0.0/tasmota/tasmota_xdrv_driver/xdrv_05_irremote.ino)
- Documentation: [Commands.md](https://github.com/tasmota/docs/blob/65430c465f/docs/Commands.md) (master at 65430c4, 2026-11-13), [Upgrading.md](https://github.com/tasmota/docs/blob/master/docs/Upgrading.md)

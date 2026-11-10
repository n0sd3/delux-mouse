# Delux M900 Pro 8K — vendor HID protocol (static RE of DELUX.exe)

Source: `DELUX.exe` sha256 `2d1f3770…664715f` (PE32, image base 0x400000), `hiddriver_1.dll`, `res/` XML.
Static analysis only (Ghidra decompile + objdump). Nothing was executed, nothing was tested on hardware.
Legend: **[C]** = confirmed in code, **[I]** = inferred, **[?]** = unknown.

## 1. Device matching

| Fact | Evidence |
|---|---|
| `hid_enumerate(0x1D57, 0xFA65)` via `hidapi.dll` (not hiddriver) [C] | `FUN_00419520` |
| `hid_enumerate(0x1D57, 0xFA60)` — same logic, FA60 = the older/1K **2.4G dongle**, NOT wired [C] | `FUN_00415ef0` |
| Command handle ("handle"): collection `usage_page == 0xFF00 && usage == 1` [C] | `FUN_00419520` |
| Event handle ("OSD handle"): collection `usage_page == 0x000A && usage == 0`; non-blocking, polled every 10 ms with `hid_read(h, buf, 100)` [C] | `FUN_00419520`, `FUN_0040f680`, `FUN_00419b00` |
| Up to 13 receivers; each gets its own reader thread [C] | `FUN_00419520` |
| Wired mode uses per-model PIDs through `hiddriver_*.dll` (`HidD_SetFeature`; feature dev = usage page 0x0B/usage 0, monitor dev = usage page 0x0A/usage 0, VID/PID from `Set_VIDPID`) [C] | hiddriver `FUN_10001d70`, `SetFeature` |

On macOS all collections of the interface are one hidapi device: write report ID 4, read report ID 3.

### Model selection (receiver is generic; mouse model comes from the event report)

First event report `03 <model> …` selects the page (`FUN_00419b00` → `FUN_00419d30`) [C]:

| byte1 | page | name in lan/EN.xml | wired PID |
|---|---|---|---|
| 0x21 | MS_3 | M600 Pro 8K | FA72 |
| **0x30** | **MS_4** | **M900 Pro 8K** | FA73 |
| 0x32 | MS_6 | M800Pro 3395 | FA75 |
| 0x34 | MS_8 | M800PRO+ | FA77 |
| 0x37 | MS_11 | M500PRO | FA7A |
| 0x38 | MS_12 | M300PRO | FA7B |
| 0x39 | MS_13 | M700PRO | FA7C |
| 0x3A | MS_14 | M800 3395 | FA7D |
| 0x3D | MS_15 | M900 | FA24 |
| **0x3F** | **MS_16** | **M900 Pro 8K** | FA26 |
| 0x29 | MS_17 | M800 Mini | FA29 |
| **0x50** | **MS_21** | **M900 Pro 8K** | 2250 |
| 0x56 | MS_24 | M300 E-Sport | 2256 |

Three "M900 Pro 8K" variants exist. **Which one the user's mouse is cannot be decided statically [?]** — read report ID 3 and look at byte 1 (0x30 / 0x3F / 0x50). Framing and commands are identical for all three; only the DPI packet (cmd 0x04) layout and the battery event type differ (noted below).

## 2. Framing (receiver FA65, "8K" mode) [C]

Host → device: `hid_write(handle_FF00, buf, 0x41)` — output report, **report ID 0x04**. No feature reports in this mode.
App passes 65 bytes; the report is 64 (ID + 63) → on macOS send 64 bytes, zero padded.

Inner packet = the wired-mode feature report (`cmd, len, …`), wrapped (`FUN_0045e200` @0x45e5df–0x45e6f6):

| off | value |
|---|---|
| 0 | 0x04 (report ID) |
| 1 | N + 5 (N = inner length) |
| 2 | 0x00 (chunk index) |
| 3 … 3+N-1 | inner packet |
| 3+N | checksum high |
| 4+N | checksum low |
| rest | 0x00 |

Checksum = 16-bit sum of bytes `[0 .. 3+N)` (includes 04, length, index), **big-endian**.

Inner packet general shape: `[0]=cmd, [1]=inner length N, [2]=0x01, payload…` (N counts from byte 0; trailing bytes inside N may be zero padding).

Long packet (only cmd 0x09, N = 0x6F = 111) is split in 2 frames (@0x45e3d7–0x45e5cd) [C]:

| frame | [0] | [1] | [2] | data | checksum (BE, sum of all preceding bytes) |
|---|---|---|---|---|---|
| 0 | 04 | 0x40 | 00 | inner[0..59) at [3..62) | [62],[63] |
| 1 | 04 | 0x39 | 01 | inner[59..111) at [3..55) | [55],[56] |

Timing [C]: `Sleep(200)` after each chunk; `Sleep(500)` after every command. No retry, no ack wait in this mode.

Device → host: **only report ID 3** on the event collection is ever read. There is **no response to commands and no read-back** in the app (see §5).

Other transports for the same inner packets [C]:
- Wired: `SetFeature(inner, N)` (report ID = cmd byte). cmd 0x09 re-chunked as `09 40 <macroIdx> <chunk> …` ×2, 64 bytes each, Sleep(200).
- FA60 dongle: `hid_send_feature_report(h, inner, N)`, repeated every 500 ms up to 10× until event type 0x50 with value 0 arrives (ack). cmd 0x09 as 3 chunks `09 40 …`, last one `09 0C …`, Sleep(1000).

## 3. Events (report ID 3) [C]

`FUN_0041a160` → per-page handler (`FUN_004a1110` MS_21, `FUN_004683a0` MS_16, `FUN_004d8880` MS_4).

| off | meaning |
|---|---|
| 0 | 0x03 |
| 1 | model code (0x30 / 0x3F / 0x50 …) |
| 2 | event type |
| 3 | value low (v0) |
| 4 | value high (v1) |

Any event with matching model code = "mouse online"; app marks offline after 6000 ms without one [C] (so the receiver sends periodic events; period itself [?]).

| type | meaning |
|---|---|
| 0x10 | current DPI stage changed: v0 = stage 1..8 [C] |
| 0x20 | polling rate changed: `(v0 >> 4) & 0xF` = rate code 1..7 (see cmd 0x06) [C] |
| 0x40 (MS_16) / 0x41 (MS_21) | battery/charge [C]: v0 == 3 → charging; v0 == 2 → shown as 100 % (charge complete [I]); otherwise v1 = percent 1..100. MS_21 shows v1 raw; MS_15 class applies a display fudge (+5..+30). MS_4 type value not extracted [?] |
| 0x50 | v0 == 0 → command ack flag (only used by FA60 path) [C] |
| 0x60 | v = 1..10 stored (purpose [?], likely coarse battery level) |
| 0x70 | v < 8 → stored into config (MS_21: light mode index) [C]/[I] |
| 0x80 | v 1..5 → only triggers profile save [?] |

There is **no battery query command**; battery is push-only.

## 4. Commands (inner packets)

Sent by "Apply" (`FUN_004a1550` MS_21, `FUN_004687e0` MS_16, `FUN_004d8db0` MS_4), order: 0x0C (if reset) → 0x04 → 0x05 → 0x06 → 0x08 → 0x09…

### 0x04 — DPI + sensor options, N = 0x38 [C] (`FUN_004a1900` MS_21, `FUN_00453230` MS_16, `FUN_004d0230` MS_4)

| off | MS_21 (0x50) | MS_16 (0x3F) / MS_4 (0x30) |
|---|---|---|
| 0,1,2 | 04 38 01 | 04 38 01 |
| 3 | (LOD << 4) \| ripple | LOD |
| 4 | (angleSnap << 4) \| motionSync | ripple control |
| 5 | DPI stage enable bitmask (bit i = stage i+1) | same |
| 6 | sensor mode (0 High Performance, 1 Low Power, 2 Office(hidden), 3 Gaming) | angle snap |
| 7 | 0 | motion sync |
| 8..15 | low byte of (DPI/50 − 1), stages 1..8 | same |
| 16..23 | high byte of (DPI/50 − 1), stages 1..8 | same |
| 24 | current stage, 1-based | same |
| 25..48 | 8 × RGB stage colour | same |
| 49 | 0x02 | 0x03 |
| 50,51 | checksum BE = sum of bytes [3..50) | same |
| 52..55 | 0 | 0 |

LOD: 0 = 1 mm, 1 = 2 mm. ripple/angle/motion: 0 = off, 1 = on. DPI slider range 1..520 → 50..26000 step 50 (XML). A stage with slider value 0 leaves its bytes 0.
Field order for MS_16/MS_4 (which struct field is which option) is [I] from the MS_21 UI mapping (same struct offsets 0x8B4/0x944/0x948/0x94C).
Defaults (MS_21): 400/800/1200/1600/3200/5000, stages 1-6 enabled, current = 2, LOD 1 mm, ripple on, angle off, motion sync on, mode Gaming.

### 0x05 — light / power / debounce, N = 0x0F [C]

| off | value |
|---|---|
| 0,1,2 | 05 0F 01 |
| 3 | (lightMode & 0x0F) << 4 |
| 4 | (sleepTimer & 0xF0) \| (9 − breathSpeed) |
| 5 | ((sleepTimer & 0x0F) << 4) \| (brightness if lightMode ∈ {1,9} else 8) |
| 6,7,8 | light colour R,G,B |
| 9 | light-off timer (slider 1..60) |
| 10 | key debounce / "Key Response Time" (slider 1..50, default 6; ms [I]) |
| 11,12 | checksum BE = sum of bytes [3..11) |

sleepTimer slider 1..60 (minutes per label text, default 10). brightness 1..8, breathSpeed 4..8. lightMode = combo index 0..11 (0 LED Off, 1 Static, 2 Breathing, 3 Neon, …).

### 0x06 — polling rate, N = 0x09 [C]

`06 09 01 B ~B 00 00 00 00`, `B = (hsCode << 4) | lowCode`, byte 4 = bitwise NOT of B (no sum checksum).
- hsCode 1..7 = 125, 250, 500, 1000, 2000, 4000, 8000 Hz (`btn_prate_hs_1..7`)
- lowCode 1..4 = 125, 250, 500, 1000 Hz (`btn_prate_1..4`)
Both nibbles are always sent; that the 8K receiver uses the high nibble is [I] (event 0x20 reports the high nibble). UI rule [C] (`FUN_00460570`): on the 8K receiver 2000/4000/8000 are only selectable when sensor mode == 3 (Gaming); otherwise capped at 1000.
Example frame, 8000 Hz: `04 0E 00 06 09 01 74 8B 00 00 00 00 01 21`.

### 0x08 — button map, N = 0x3B [C] (`FUN_004a1c70`)

`08 3B 01`, then 18 × 3 bytes at [3..57), checksum BE = sum [3..57) at [57],[58].
Default order (UI ids): b1 left, b2 right, b3 middle, b4 DPI cycle, b5 off, b6 0x18, b7 forward, b8 backward, b9..b16 off, b17 scroll up, b18 scroll down.

Function codes (table @0x51CE40, identical for MS_16 @0x519D98, MS_4 @0x51FB60):

| function | bytes | function | bytes |
|---|---|---|---|
| disabled | 01 00 00 | media player | 15 00 00 |
| left | 02 00 00 | prev track | 16 00 00 |
| right | 03 00 00 | next track | 17 00 00 |
| middle | 04 00 00 | play/pause | 18 00 00 |
| backward | 05 00 00 | stop | 19 00 00 |
| forward | 06 00 00 | mute | 1A 00 00 |
| double click | 07 00 00 | vol up | 1B 00 00 |
| fire button | 08 00 00 | vol down | 1C 00 00 |
| scroll up | 09 00 00 | calculator | 1D 00 00 |
| scroll down | 0A 00 00 | email | 1E 00 00 |
| tilt left | 0B 00 00 | browser fwd | 20 00 00 |
| tilt right | 0C 00 00 | browser back | 21 00 00 |
| DPI cycle | 0D 00 00 | browser stop | 22 00 00 |
| DPI up | 0E 00 00 | my computer | 23 00 00 |
| DPI down | 0F 00 00 | browser refresh | 24 00 00 |
| easy aim | 10 00 03 | browser home | 25 00 00 |
| LED loop | 29 00 03 | browser search | 26 00 00 |
| keyboard | 11 mod key | macro | 12 00 idx |

Keyboard: `11 <HID modifier bitmask> <HID usage>`; presets: cut 11 01 1B, copy 11 01 06, paste 11 01 19, open 11 01 12, save 11 01 16, find 11 01 09, redo 11 01 1C, select-all 11 01 04, print 11 01 13, close window 11 04 3D, swap windows 11 04 2B, show desktop 11 08 07, run 11 08 15, lock 11 08 0F, screenshot 11 0A 16, favourites 11 03 12. Other table entries without menu names: 13, 14, 2B, 34, 35, 36, 3C, 28, 3A (UI ids 0x13,0x73,0x15,0x36,0x37,0x38,0x34,0x18,0x19) [?].

### 0x09 — macro, N = 0x6F [C structure, I semantics] (`FUN_0045f3e0`)

| off | value |
|---|---|
| 0,1 | 09 6F |
| 2 | macro block index |
| 3 | key struct +0x0C [?] |
| 4,5,6 | key struct +0x43C..0x43E [?] |
| 7 | key struct +0x40, min 1 (repeat count [I]) |
| 8 | unit count of events in this block |
| 9.. | events |
| 0x6D,0x6E | checksum BE = sum [3..0x6D) |

Events: key `[(up ? 0x81 : 0x01) | delay, code]` when delay < 0x80; `[flag | delay%100, code, delay/100, 03]` otherwise; type-0 action `[delay, F6, v1hi, v1lo, v2hi, v2lo]`. If events exceed 0x62 bytes the block ends with `[idx+1, FE]` and continues in the next block. Not fully decoded.

### 0x0C — factory reset [C bytes, I meaning], N = 0x0A

`0C 0A 01 FE 01 FE 00 00 00 00` — sent first when the "reset" flag is set, then every section is re-sent with defaults. Frame: `04 0F 00 0C 0A 01 FE 01 FE 00 00 00 00 02 27`.

## 5. Not present / not determined

- **No read commands at all** in the receiver path: no "read config", no firmware-version query, no battery query. The app keeps profiles in a local file (`ms_21\pro.data`) and only pushes; device → host data is limited to the report ID 3 events. A macOS tool cannot read DPI/polling settings back with what this app uses [C that the app does not; whether firmware supports it is ?].
- No handshake/unlock before commands [C: none sent].
- `GetFeature` is imported from hiddriver but usage on wired path was not traced.
- Which model code the user's mouse reports (0x30/0x3F/0x50) [?].
- MS_4 battery event type and its cmd 0x05/0x06 field details were only spot-checked (same constants `05 0F 01`, `06 09 01`, `0C 0A 01 FE`).
- Whether the receiver accepts a 64-byte write vs the app's 65-byte buffer; event period; macro semantics.

## 6. Key functions

| addr | role |
|---|---|
| 0x419520 / 0x415ef0 | open FA65 / FA60 receivers |
| 0x419b00, 0x41a160 | event read, model detect, dispatch |
| 0x45e200 (MS_15/17/21/24), 0x452770 (MS_14/16), 0x4cf440 (MS_3/4) | transport/framing (identical after normalisation) |
| 0x4a1550, 0x4a1900, 0x4a1c70, 0x45f3e0 | MS_21 apply, DPI, buttons, macro |
| 0x4687e0, 0x453230, 0x468b90, 0x453590 | MS_16 equivalents |
| 0x4d8db0, 0x4d0230, 0x4d9140 | MS_4 equivalents |
| 0x4592f0, 0x4a2ba0, 0x49d260, 0x49fcc0 | config→UI map, defaults, control binding, menu→function id |

---

## Variante 0x30 (MS_4) — layouts finais

Fonte: decompilação Ghidra de `FUN_004d0230` (DELUX.exe sha256 `2d1f3770…664715f`), 2026-11-10.
Onde esta seção divergir do resto do documento, vale esta seção.

### Cmd 0x04 — confirmado no código

| offset | conteúdo | origem no struct de config |
|---|---|---|
| 0–2 | `04 38 01` | constante |
| 3 | LOD (0 = 1 mm, 1 = 2 mm) | `+0x8b4` |
| 4 | ripple (0/1) | `+0x940` |
| 5 | bitmask de estágios ativos, bit i = estágio i+1 | `+0x904 … +0x920` (`== 1`) |
| 6 | angle snap (0/1) | `+0x944` |
| 7 | motion sync (0/1) | `+0x948` |
| 8–15 | byte baixo de (valor − 1), estágios 1..8 | `+0x8c4 …` (int32 cada) |
| 16–23 | byte alto de (valor − 1) | idem |
| 24 | estágio atual, 1-based (guardado 0-based, envia +1) | `+0x934` |
| 25–48 | 8 × RGB | `+0x960 …` (passo 4) |
| 49 | `0x03` constante | — |
| 50, 51 | checksum16 dos bytes [3..50), big-endian | — |
| 52–55 | zero | — |

- "valor" = DPI/50 (slider 1..520), conforme o resto da spec.
- Bytes 3, 4, 6, 7: os **offsets** são confirmados; o **nome** de cada campo (LOD/ripple/angle/motion) vem do mapeamento de UI da variante MS_21, que usa os mesmos offsets de struct. Confirmar no hardware.
- **Não existe campo "modo do sensor" nesta variante.** A regra "polling > 1000 Hz exige modo Gaming" não se aplica.
- Transporte: `FUN_004cf440(this, buf, 0x38)`.

### Não reanalisado para MS_4 (vale o layout comum; confirmar no hardware)

- `0x06` polling: qual nibble o receptor 8K usa — determinar pelo evento `0x20` no teste.
- `0x05`, `0x08`: constantes conferidas por amostragem na análise original; ordem dos 18 slots a levantar no hardware.
- Evento de bateria: tipo `0x40` medido no hardware (v0 = 1 descarregando, 84 %).

### Verificação no hardware — 2026-11-10 (mouse do usuário, modelo 0x30, receptor FA65)

| comando | resultado |
|---|---|
| `0x04` DPI/estágios | **confirmado**: 400/3200/12800 → três velocidades claramente distintas; ciclo de 3 estágios (eventos `DpiStage` 2→3→1) |
| `0x04` LOD / ripple / angle snap / motion sync | enviados sem erro; efeito **não verificado** individualmente |
| `0x05` luz | **confirmado**: modo 1 = vermelho estático |
| `0x05` sleep / debounce | **não verificado** |
| `0x06` polling | enviado sem erro; **não verificado**. Nenhum evento `0x20` em resposta; medir taxa via hidapi no macOS não distingue (70–160 Hz em qualquer ajuste — o SO agrupa relatórios) |
| `0x08` botões | **confirmado**: slot 6 = lateral da frente, slot 7 = lateral de trás, slot 3 = botão DPI. Mouse tem só esses 3 botões além de esq/dir/meio/scroll |
| `0x08` tecla `11 00 68` | **confirmado**: lateral de trás emite HID usage 0x68 (F13) na interface de teclado do receptor |
| evento bateria | **confirmado**: v0 = 1 descarregando, v0 = 3 carregando (cabo conectado). v0 = 2 (cheio) não observado. Após tirar o cabo a porcentagem oscila alguns segundos (100 → 86 → 84) |
| `0x0C` reset | **sem efeito observável sozinho**: DPI, botão remapeado e luz ficaram como estavam. "Restaurar fábrica" = `0x0C` + reenviar todas as seções com os padrões do driver |

### Correção 2026-11-10 — polling e eventos da variante 0x30 (MS_4)

Fonte: segunda análise estática do driver (relatório completo em `.re/readback-driver-report.md`, fora do git).

- **Cmd `0x06` em MS_4 usa código one-hot**, não `(hs<<4)|low` (código de aplicar em `0x4d8fef`):

  | taxa | 125 | 250 | 500 | 1000 | 2000 | 4000 | 8000 Hz |
  |---|---|---|---|---|---|---|---|
  | B | 0x20 | 0x10 | 0x08 | 0x04 | 0x02 | 0x01 | 0x40 |

  Pacote: `06 09 01 B ~B 00 00 00 00`. Os testes de polling de 2026-11-10 usaram a codificação antiga (ex.: `0x44` para 1000 Hz) — valores inválidos para este modelo; efeito no mouse desconhecido. **Ainda não verificado no hardware com a codificação correta.**
- Evento `0x20`: v0 = mesmo código one-hot (taxa atual).
- Evento `0x70`: v0 = índice do modo de luz atual (0..7), o mesmo campo do byte 3 do cmd `0x05`.
- Evento `0x60`: 1..10, provavelmente nível grosso de bateria (inferido). Evento `0x80`: 1..5, significado não determinado.
- **Não existe leitura de configuração**: o driver não tem nenhum comando de consulta em nenhum transporte (sem `hid_get_feature_report`, `GetFeature` resolvido mas nunca chamado, report 4 de entrada nunca lido). O web driver da Delux (`controlhub.top`) é de outra família (Compx, VID 3554, report ID 8) e não serve para este receptor.

### Verificação no hardware — 2026-11-10, segunda rodada

| item | resultado |
|---|---|
| `0x06` polling (código one-hot) | **confirmado** por medição da taxa de relatórios via hidapi: 125 → 128 Hz, 500 → 508 Hz, 1000 → 816 Hz (pico em janela de 250 ms, com movimento contínuo). 2000/4000/8000 Hz: não mensurável pelo Mac |
| `0x05` em MS_4 | mesmo layout de MS_21, mas: byte 9 = constante `2` (sem controle na página MS_4); sleep 1..20 min; debounce 1..25 em passos de 2 ms (o driver mostra 2 × valor) |
| `0x05` sleep | **confirmado** com byte 9 = 2: sleep = 1 → sinais a cada 2 s até ~50 s, depois espaçados (~4 s), silêncio total a partir de ~115 s. Com byte 9 = 10 (valor antigo) o mouse não dormiu em 200 s |
| evento `0x10` → app | **confirmado**: botão de DPI move a seleção "Atual" e o cabeçalho |
| luz segue DPI | **confirmado**: app reenvia `0x05` com a cor do estágio ao receber `0x10` |
| LOD, ripple, angle snap, motion sync, debounce | enviados como o driver envia; efeito não confirmado (só perceptível por sensação) |

## Macros (cmd 0x09) e modo cabo — variante 0x30, 2026-11-10

Relatórios completos da análise (fora do git): `.re/macros-ms4-report.md`, `.re/wired-ms4-report.md`, `.re/cmd05-ms4-report.md`.

### Cmd 0x09 — macro (uma por botão)

`09 6F <slot+1> <modo> 00 00 00 <vezes> <n registros> <registros…> <sum16 BE de [3..0x6D)>`, 111 bytes.

- Registro = `[flag | atraso/10, código]`; flag `0x01` pressiona, `0x81` solta. Como o bit 0 está sempre ligado, só existem atrasos de 10, 30, 50 … 1270 ms (atraso é a espera **depois** da ação).
- Código = HID usage de teclado (modificadores são teclas comuns E0..E7); botões do mouse: F1 esq., F2 dir., F3 meio, F4 avançar, F5 voltar. Código `03` é marcador de atraso longo (não usado pelo app).
- Modo: 0 = tocar N vezes (byte 7), 1 = repetir até qualquer tecla, 2 = repetir enquanto segura.
- Driver permite 46 ações. Botão aponta para a macro com `12 00 <slot+1>` no cmd 0x08; driver envia 0x08 antes de 0x09.
- **Confirmado no hardware**: macro "a, b" e depois "c, d" no lateral de trás (slot 7 → byte 8), modo 0, 1 vez.

### Modo cabo

- Mouse com a chave em **OFF** e cabo ligado enumera como `1D57:FA73` "USB Gaming Mouse", full-speed (12 Mb/s). Com a chave em ON o cabo só carrega e o mouse segue no receptor.
- Comandos = feature reports na coleção usage page `0x0B`; o ID do report é o byte de comando e o conteúdo é o pacote interno **sem** o envelope `04 len idx … checksum`. Tamanhos do descritor (ID incluído): 04 → 52, 05 → 13, 06 → 9, 08 → 59, 0C → 6 (o pacote interno é cortado nesse tamanho; o resto era preenchimento).
- Macro no cabo: dois reports de 64 bytes `09 40 <slot+1> <pedaço 0|1>` + 60 bytes de `inner[3 + 60·pedaço…]`, 200 ms entre eles.
- Polling limitado a 1000 Hz (o driver nunca envia mais no cabo).
- Eventos: os mesmos `03 30 …` na mesma interface; bateria chega como "carregando".
- **Leitura não existe**: GET_REPORT nos feature reports 04/05/06/07/08/0A/0B/0C/10 dá timeout; A0 devolve zeros.
- **Confirmado no hardware**: DPI (400 → lento), reenvio do perfil completo, macro nova ("cd") gravada pelo cabo; app mostra "Conectado por cabo".
- O descritor declara outros feature reports (22..2E) que o driver nunca usa. Não tocar.

### Outros achados

- Página MS_4 envia `0C 0A 01 FE 01 FE…` no início de **todo** Aplicar (parece preâmbulo, não reset). O app não envia e os comandos funcionam sem ele.

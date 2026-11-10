# Delux M900 PRO para macOS — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** App macOS (barra de menu + janela) que mostra a bateria do Delux M900 PRO 8K, configura sensor/botões/luz/energia e dispara scripts pelos botões.

**Architecture:** Workspace Cargo com núcleo de protocolo puro (`delux-proto`), transporte HID (`delux-hid`), perfil local (`delux-profile`), serviço de dispositivo (`delux-core`), CLI de diagnóstico (`delux-cli`) e app Tauri 2 (`app/`). Cada comando do firmware só entra na UI depois de confirmado no hardware via CLI.

**Tech Stack:** Rust estável, `hidapi` 2, `serde` + `toml`, `thiserror`, `clap` 4, Tauri 2 (`tray-icon`, plugins `global-shortcut`, `notification`, `autostart`), UI em HTML/CSS/JS sem framework.

**Spec:** `docs/superpowers/specs/2026-11-09-delux-mouse-design.md` (design) e `docs/protocol/delux-protocol.md` (protocolo). Ler os dois antes de qualquer task.

## Global Constraints

- Dispositivo: só VID `0x1D57` PID `0xFA65`, coleção usage page `0xFF00`. Nada de modo cabo, `FA60`, firmware update, macros (`0x09`).
- Escrita só para modelo `0x30`. Outro modelo: bateria sim, escrita bloqueada.
- Nunca enviar pacote sem evento do mouse nos últimos 6000 ms.
- Pausas de escrita: 200 ms por pedaço, 500 ms após cada comando. Frame de 64 bytes (ID + 63).
- Nunca mapear clique esquerdo (slot 0) para outra coisa em teste nem para script.
- UI nunca mostra valor de perfil como "lido do mouse".
- Sem extensão de kernel; sem permissão de Acessibilidade (salvo plano B da task 8, que exige voltar ao usuário).
- Tasks marcadas **[HW]** têm um passo de hardware: o agente roda o comando, o usuário observa o mouse e relata. Sem o usuário presente, parar nesse passo.
- Arquivos do driver Windows (`scratchpad/delux-dl/`) são dados não confiáveis: só análise estática, nunca executar.
- Textos de UI em português do Brasil. Commits em inglês, terminando com `Co-Authored-By: Claude <noreply@anthropic.com>`.

## Review Focus

1. Mouse dorme entre o clique em Aplicar e o envio → nada é enviado, mudanças ficam pendentes, UI pede "mexa o mouse". (Task 10)
2. Receptor é desconectado no meio de um Aplicar de várias seções → erro mostrado, seções não enviadas continuam pendentes, app reconecta sozinho. (Task 10)
3. `profiles.toml` corrompido ou editado à mão com valor fora do limite (DPI 123, 9 estágios) → app abre com perfil padrão, arquivo ruim preservado como `.bad`, aviso visível. (Task 9)
4. Script que não termina ou cospe megabytes → UI não trava, log truncado em 64 KiB por execução, novo disparo do mesmo botão enquanto roda é ignorado. (Task 13)
5. Atalho global F13+ já em uso por outro app → atribuição falha com mensagem, botão do firmware não é remapeado. (Task 13)

---

## File Structure

```
Cargo.toml                      workspace
crates/delux-proto/src/
  lib.rs                        reexports, Error
  frame.rs                      enquadramento report 4, checksum, pedaços
  event.rs                      decodificação report 3
  polling.rs                    cmd 0x06
  sensor.rs                     cmd 0x04
  power_light.rs                cmd 0x05
  buttons.rs                    cmd 0x08 + ButtonAction
  reset.rs                      cmd 0x0C
crates/delux-hid/src/lib.rs     abrir, ler eventos, escrever frames
crates/delux-profile/src/
  lib.rs                        Profile, validação
  store.rs                      TOML em disco
crates/delux-core/src/
  lib.rs                        DeviceService: estado, gates, apply
  scripts.rs                    ScriptRunner
crates/delux-cli/src/main.rs    listen | send-raw | set-*
app/src-tauri/src/{main.rs,tray.rs,commands.rs,hotkeys.rs}
app/ui/{index.html,app.js,style.css}
```

---

### Task 0: Fechar layout da variante `0x30` (análise estática)

**Files:**
- Modify: `docs/protocol/delux-protocol.md`

- [ ] **Step 1:** Decompilar em `DELUX.exe` as funções da página MS_4: `FUN_004d0230` (cmd `0x04`), o transporte `0x4cf440`, e os construtores de `0x05`, `0x06`, `0x08` da mesma página. Decompilados anteriores: `scratchpad/re-scripts/dec/`.
- [ ] **Step 2:** Determinar, com endereço de evidência para cada um: (a) significado dos bytes 3–7 e 49 do `0x04` em MS_4; (b) se existe campo "modo do sensor"; (c) se `0x06` em MS_4 usa nibble alto, baixo ou ambos; (d) ordem dos 18 slots do `0x08` em MS_4; (e) tipo do evento de bateria em MS_4 (medido: `0x40`) e significado de v0.
- [ ] **Step 3:** Adicionar à spec uma seção "Variante 0x30 (MS_4)" com as tabelas finais, marcando cada campo como `confirmado no código` ou `não determinado`.
- [ ] **Step 4:** Se (b) for "não existe": anotar no design, seção 6.3, que a aba Sensor não tem modo do sensor e a regra > 1000 Hz não se aplica.
- [ ] **Step 5:** Commit `docs: pin MS_4 (0x30) packet layouts`.

**Produces:** tabelas de bytes usadas nas tasks 4–7. Onde este plano e a seção "Variante 0x30" divergirem, vale a seção.

---

### Task 1: Workspace + enquadramento

**Files:**
- Create: `Cargo.toml`, `.gitignore` (`target/`, `.superpowers/`), `crates/delux-proto/{Cargo.toml,src/lib.rs,src/frame.rs}`

**Produces:**
```rust
pub const REPORT_LEN: usize = 64;
pub type Frame = [u8; REPORT_LEN];
pub fn checksum16(bytes: &[u8]) -> u16;               // soma, wrapping
pub fn frames(inner: &[u8]) -> Result<Vec<Frame>, Error>; // 1 frame se len<=59, senão pedaços de 59
#[derive(thiserror::Error)] pub enum Error { EmptyPacket, PacketTooLong(usize), OutOfRange{field:&'static str, value:i64} }
```

- [ ] **Step 1:** `git init` em `~/delux-mouse`; criar workspace.
- [ ] **Step 2: Testes que falham** em `frame.rs`:

```rust
#[test] fn polling_8k_vector() {
    let f = frames(&[0x06,0x09,0x01,0x74,0x8B,0,0,0,0]).unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(&f[0][..14], &[0x04,0x0E,0x00,0x06,0x09,0x01,0x74,0x8B,0,0,0,0,0x01,0x21]);
    assert!(f[0][14..].iter().all(|b| *b == 0));
}
#[test] fn chunked_111_bytes() {
    let inner: Vec<u8> = (0..111u8).collect();
    let f = frames(&inner).unwrap();
    assert_eq!(f.len(), 2);
    assert_eq!(&f[0][..3], &[0x04,0x40,0x00]);
    assert_eq!(&f[0][3..62], &inner[..59]);
    assert_eq!(u16::from_be_bytes([f[0][62],f[0][63]]), checksum16(&f[0][..62]));
    assert_eq!(&f[1][..3], &[0x04,0x39,0x01]);
    assert_eq!(&f[1][3..55], &inner[59..]);
    assert_eq!(u16::from_be_bytes([f[1][55],f[1][56]]), checksum16(&f[1][..55]));
}
#[test] fn empty_is_error() { assert!(matches!(frames(&[]), Err(Error::EmptyPacket))); }
```

- [ ] **Step 3:** `cargo test -p delux-proto` → FAIL (não compila).
- [ ] **Step 4:** Implementar `checksum16` e `frames`. Byte 1 = tamanho do pedaço + 5; byte 2 = índice do pedaço.
- [ ] **Step 5:** `cargo test -p delux-proto` → PASS.
- [ ] **Step 6:** Commit `feat(proto): report-4 framing`.

---

### Task 2: Eventos

**Files:** Create `crates/delux-proto/src/event.rs`

**Produces:**
```rust
pub const MODEL_M900PRO_8K_MS4: u8 = 0x30;
pub enum BatteryState { Discharging, Charging, Full, Unknown(u8) }
pub enum EventKind { Battery{state:BatteryState, percent:u8}, DpiStage(u8), PollingRate(u8), Ack(u8), Other{kind:u8,v0:u8,v1:u8} }
pub struct Event { pub model: u8, pub kind: EventKind }
pub fn parse_event(report: &[u8]) -> Option<Event>;  // None se len<5 ou report[0]!=3
```

- [ ] **Step 1: Testes que falham** (capturas reais):

```rust
#[test] fn battery_84() {
    let e = parse_event(&[0x03,0x30,0x40,0x01,0x54]).unwrap();
    assert_eq!(e.model, 0x30);
    assert!(matches!(e.kind, EventKind::Battery{state:BatteryState::Discharging, percent:84}));
}
#[test] fn dpi_stage_3() { assert!(matches!(parse_event(&[0x03,0x30,0x10,0x03,0x00]).unwrap().kind, EventKind::DpiStage(3))); }
#[test] fn charging() { assert!(matches!(parse_event(&[3,0x30,0x40,3,10]).unwrap().kind, EventKind::Battery{state:BatteryState::Charging,..})); }
#[test] fn full_is_100() { assert!(matches!(parse_event(&[3,0x30,0x40,2,0]).unwrap().kind, EventKind::Battery{state:BatteryState::Full, percent:100})); }
#[test] fn polling() { assert!(matches!(parse_event(&[3,0x30,0x20,0x70,0]).unwrap().kind, EventKind::PollingRate(7))); }
#[test] fn rejects_short_and_wrong_id() { assert!(parse_event(&[3,0x30]).is_none()); assert!(parse_event(&[4,0,0,0,0]).is_none()); }
```

Mapa: tipo `0x40` e `0x41` = bateria (v0: 1→Discharging, 3→Charging, 2→Full com percent 100, outro→Unknown; percent = v1 limitado a 100); `0x10` DpiStage(v0); `0x20` PollingRate((v0>>4)&0xF); `0x50` Ack(v0).

- [ ] **Step 2:** rodar → FAIL. **Step 3:** implementar. **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(proto): decode report-3 events`.

---

### Task 3: Transporte HID + CLI `listen` / `send-raw` **[HW]**

**Files:** Create `crates/delux-hid/{Cargo.toml,src/lib.rs}`, `crates/delux-cli/{Cargo.toml,src/main.rs}`

**Consumes:** `Frame`, `parse_event`, `Event`.

**Produces:**
```rust
pub const VID: u16 = 0x1D57; pub const PID: u16 = 0xFA65;
pub enum HidError { NotFound, Io(String) }
pub struct Device { /* hidapi::HidDevice */ }
impl Device {
    pub fn open() -> Result<Device, HidError>;                    // 1ª entrada VID/PID com usage_page 0xFF00
    pub fn read_event(&self, timeout_ms: i32) -> Result<Option<Event>, HidError>;
    pub fn write_frames(&self, frames: &[Frame]) -> Result<(), HidError>; // sleep 200 ms após cada frame, +500 ms no fim
}
```
CLI (binário `delux`): `delux listen [--secs N]` imprime eventos decodificados + hex; `delux send-raw <hex do pacote interno>` exige evento nos últimos 6 s (escuta até 10 s), imprime os frames em hex, pede `--yes` para enviar de fato.

- [ ] **Step 1:** Implementar `Device` (referência funcional: `scratchpad/spike-listen/src/main.rs`) e a CLI. Teste unitário só do parser de hex do `send-raw` (`"06 09 01"` e `"060901"` → mesmos bytes; hex ímpar → erro).
- [ ] **Step 2:** `cargo test --workspace` → PASS.
- [ ] **Step 3 [HW]:** `cargo run -p delux-cli -- listen --secs 10` com o usuário mexendo o mouse. Esperado: linhas `Battery … NN%` a cada ~2 s com `model=0x30`.
- [ ] **Step 4:** Commit `feat: hid transport and diagnostic cli`.

---

### Task 4: Polling `0x06` **[HW]**

**Files:** Create `crates/delux-proto/src/polling.rs`; Modify `crates/delux-cli/src/main.rs`

**Produces:**
```rust
pub enum PollingRate { Hz125=1, Hz250, Hz500, Hz1000, Hz2000, Hz4000, Hz8000 }
pub fn encode_polling(rate: PollingRate) -> Vec<u8>; // [06,09,01,B,!B,0,0,0,0]
```
`B` conforme Task 0(c); na falta de determinação: `(code<<4) | min(code,4)`.

- [ ] **Step 1: Teste que falha:** `encode_polling(Hz8000)` → `[0x06,0x09,0x01,0x74,0x8B,0,0,0,0]`; para toda taxa, `p[3] ^ p[4] == 0xFF` e `len == 9`.
- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** CLI `delux set-polling <hz> [--yes]`.
- [ ] **Step 6 [HW]:** enviar 1000 Hz → o usuário confirma mouse normal. Enviar 500 Hz → `listen` deve mostrar `PollingRate(3)` e/ou o usuário mede em site de teste de polling. Voltar para a taxa que ele usa. Se nenhum efeito observável: parar, voltar à Task 0(c).
- [ ] **Step 7:** Registrar resultado na spec ("Variante 0x30": `0x06` confirmado no hardware em 2026-…). Commit `feat(proto): polling rate command`.

---

### Task 5: Sensor/DPI `0x04` **[HW]**

**Files:** Create `crates/delux-proto/src/sensor.rs`; Modify CLI

**Produces:**
```rust
pub struct Rgb(pub u8, pub u8, pub u8);
pub struct DpiStage { pub enabled: bool, pub dpi: u16, pub color: Rgb }
pub enum Lod { Mm1 = 0, Mm2 = 1 }
pub struct SensorConfig {
    pub stages: [DpiStage; 8], pub current_stage: u8, // 1..=8, tem de estar enabled
    pub lod: Lod, pub ripple: bool, pub angle_snap: bool, pub motion_sync: bool,
}
pub fn encode_sensor(cfg: &SensorConfig) -> Result<Vec<u8>, Error>; // 0x38 = 56 bytes
```
Layout (spec, coluna MS_4; ajustar pela Task 0): `[0..3]=04 38 01`, `[3]=lod`, `[4]=ripple`, `[5]=bitmask de estágios ativos (bit i = estágio i+1)`, `[6]=angle_snap`, `[7]=motion_sync`, `[8..16]=low(dpi/50-1)`, `[16..24]=high(dpi/50-1)`, `[24]=current_stage`, `[25..49]=8×RGB`, `[49]=0x03`, `[50..52]=checksum16([3..50]) BE`, resto zero.

- [ ] **Step 1: Testes que falham:**

```rust
#[test] fn dpi_encoding() { // 1600 → 31 = 0x001F ; 26000 → 519 = 0x0207
    let mut c = SensorConfig::default_profile(); c.stages[0].dpi = 1600; c.stages[1].dpi = 26000;
    let p = encode_sensor(&c).unwrap();
    assert_eq!(p.len(), 0x38); assert_eq!(&p[..3], &[0x04,0x38,0x01]);
    assert_eq!((p[8], p[16]), (0x1F, 0x00)); assert_eq!((p[9], p[17]), (0x07, 0x02));
    assert_eq!(p[49], 0x03);
    assert_eq!(u16::from_be_bytes([p[50],p[51]]), checksum16(&p[3..50]));
}
#[test] fn rejects_bad_dpi() { for bad in [0u16, 49, 75, 26050] { /* dpi=bad → Err(OutOfRange{field:"dpi",..}) */ } }
#[test] fn rejects_current_stage_disabled_or_zero() { /* current_stage 0, 9, ou estágio desligado → Err */ }
#[test] fn rejects_no_stage_enabled() { /* todos enabled=false → Err */ }
```
`SensorConfig::default_profile()`: 6 estágios ativos 400/800/1600/3200/6400/12800, estágios 7–8 desligados (16000/26000), cores distintas, `current_stage=3`, LOD 1 mm, tudo mais desligado.

- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** CLI `delux set-dpi <d1,d2,…> --stage N [--yes]` (demais campos do default).
- [ ] **Step 6 [HW]:** enviar `400,3200,12800 --stage 1`. o usuário: cursor muito lento; botão DPI → rápido → muito rápido → volta; `listen` mostra `DpiStage` 1→2→3→1. Depois testar um por vez, com o usuário relatando: LOD 2 mm, angle snap ligado (linhas retas "grudam"). Se o comportamento não bater: parar, não tentar variações às cegas — voltar à Task 0.
- [ ] **Step 7:** Registrar na spec; commit `feat(proto): sensor and dpi command`.

---

### Task 6: Luz/energia/debounce `0x05` **[HW]**

**Files:** Create `crates/delux-proto/src/power_light.rs`; Modify CLI

**Produces:**
```rust
pub struct PowerLightConfig {
    pub light_mode: u8,        // 0..=15, nomes dos modos: tabela da spec
    pub breath_speed: u8,      // 1..=8
    pub brightness: u8,        // 1..=8 (só usado se light_mode é 1 ou 9; senão envia 8)
    pub color: Rgb,
    pub light_off_min: u8,     // 1..=60
    pub sleep_min: u8,         // 1..=60
    pub debounce_ms: u8,       // 1..=50, default 6
}
pub fn encode_power_light(cfg: &PowerLightConfig) -> Result<Vec<u8>, Error>; // 0x0F = 15 bytes
```
Limites de `light_mode`, `breath_speed` e `brightness` acima são provisórios: usar os da Task 0 / spec se diferirem.

Layout: `05 0F 01`, `[3]=mode<<4`, `[4]=(sleep & 0xF0) | (9 - breath_speed)`, `[5]=((sleep & 0x0F)<<4) | brightness_efetivo`, `[6..9]=RGB`, `[9]=light_off_min`, `[10]=debounce_ms`, `[11..13]=checksum16([3..11]) BE`, `[13..15]=0`.

- [ ] **Step 1: Teste que falha:** config `{mode:1, breath:3, brightness:5, color:(1,2,3), light_off:10, sleep:0x12, debounce:6}` → `[05,0F,01, 0x10, 0x16, 0x25, 1,2,3, 10, 6, cs_hi, cs_lo, 0, 0]` com `cs = checksum16(&p[3..11])`; mode 2 com brightness 5 → `p[5] & 0x0F == 8`; cada campo fora do limite → `Err(OutOfRange)`.
- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** CLI `delux set-power --sleep N --debounce N --light-mode N … [--yes]`.
- [ ] **Step 6 [HW]:** sleep 1 min → o usuário solta o mouse; `listen` deve ficar em silêncio após ~1 min. Trocar modo/cor de luz → o usuário descreve. Debounce: sem efeito observável → fica "não verificado". Restaurar sleep para valor que ele queira.
- [ ] **Step 7:** Registrar na spec; commit `feat(proto): power and light command`.

---

### Task 7: Botões `0x08` **[HW]**

**Files:** Create `crates/delux-proto/src/buttons.rs`; Modify CLI

**Produces:**
```rust
pub const SLOT_COUNT: usize = 18;
pub enum ButtonAction {
    Disabled, Left, Right, Middle, Back, Forward, DoubleClick, Fire, ScrollUp, ScrollDown,
    DpiCycle, DpiUp, DpiDown,
    Key { modifiers: u8, usage: u8 },   // HID modifiers bitmask + HID keyboard usage
    Media(u8),                          // 0x15..=0x1E
    Raw([u8; 3]),
}
impl ButtonAction { pub fn to_bytes(&self) -> [u8;3]; pub fn from_bytes(b:[u8;3]) -> Self; }
pub struct ButtonMap(pub [ButtonAction; SLOT_COUNT]);
impl ButtonMap { pub fn factory() -> Self; }          // padrões da spec
pub fn encode_buttons(map: &ButtonMap) -> Result<Vec<u8>, Error>; // 0x3B = 59 bytes; Err se slot 0 != Left
```
Códigos (byte 0): `01` off, `02` left, `03` right, `04` middle, `05` back, `06` forward, `07` double, `08` fire, `09`/`0A` scroll, `0D`/`0E`/`0F` DPI cycle/up/down, `11 mod usage` tecla, `15..1E` mídia. Layout: `08 3B 01`, 18×3 em `[3..57)`, `checksum16([3..57])` BE em `[57..59)`.

- [ ] **Step 1: Testes que falham:** `Key{modifiers:0, usage:0x68}.to_bytes() == [0x11,0x00,0x68]`; ida e volta `from_bytes(to_bytes(a)) == a` para todas as variantes; `encode_buttons(&ButtonMap::factory())` tem len 59, cabeçalho `08 3B 01`, checksum certo; mapa com slot 0 = `Disabled` → `Err`.
- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** CLI `delux set-button <slot> <ação> [--yes]` (envia `factory()` com esse slot alterado).
- [ ] **Step 6 [HW] mapear slots:** para cada slot 1..N, mapear para `Key{0, usage do 'a'+slot}` e o usuário diz qual botão físico digita a letra. Registrar na spec a tabela slot ↔ botão físico. Nunca tocar no slot 0.
- [ ] **Step 7 [HW] tecla inerte (design 8.5b):** mapear botão lateral para F13; rodar `delux listen-key` (subcomando novo: registra F13 como hotkey global via crate `global-hotkey` e imprime quando dispara). Esperado: disparo a cada clique. Se falhar: **parar e perguntar ao usuário** sobre o plano B (Acessibilidade); a Task 13 depende disto.
- [ ] **Step 8:** Restaurar `factory()`; registrar na spec; commit `feat(proto): button map command`.

---

### Task 8: Reset `0x0C` + estados de bateria **[HW]**

**Files:** Create `crates/delux-proto/src/reset.rs`; Modify CLI, `event.rs` se necessário

**Produces:** `pub fn encode_factory_reset() -> Vec<u8>;` → `[0x0C,0x0A,0x01,0xFE,0x01,0xFE,0,0,0,0]`

- [ ] **Step 1–3:** teste do vetor exato → FAIL → implementar → PASS.
- [ ] **Step 4 [HW] bateria:** `listen` enquanto o usuário pluga o cabo de carga e depois tira. Registrar valores reais de v0; corrigir `BatteryState` e seus testes se diferirem de 1/3/2.
- [ ] **Step 5 [HW] reset:** mudar DPI para algo óbvio, `delux reset --yes`, o usuário confirma que voltou ao normal de fábrica. Se o reset sozinho não restaurar (o driver reenviava todos os padrões depois): `reset` passa a ser `0x0C` + envio de `SensorConfig`/`PowerLight`/`ButtonMap` padrão; registrar.
- [ ] **Step 6:** Commit `feat(proto): factory reset; pin battery states`.

---

### Task 9: Perfil

**Files:** Create `crates/delux-profile/{Cargo.toml,src/lib.rs,src/store.rs}`

**Consumes:** `SensorConfig`, `PollingRate`, `PowerLightConfig`, `ButtonMap`, `ButtonAction` (todos ganham `Serialize/Deserialize/Clone/PartialEq`).

**Produces:**
```rust
pub struct ScriptBinding { pub slot: usize, pub hotkey_usage: u8, pub command: String, pub label: String }
pub struct Profile { pub name: String, pub sensor: SensorConfig, pub polling: PollingRate,
                     pub power_light: PowerLightConfig, pub buttons: ButtonMap, pub scripts: Vec<ScriptBinding> }
impl Profile { pub fn default_named(name:&str) -> Self; pub fn validate(&self) -> Result<(), Vec<String>>; }
pub enum Section { Sensor, Polling, PowerLight, Buttons }
pub fn changed_sections(old: &Profile, new: &Profile) -> Vec<Section>;
pub struct Settings { pub low_battery_pct: u8 /*20*/, pub autostart: bool /*true*/ }
pub struct Store { pub profiles: Vec<Profile>, pub active: String, pub applied_once: bool, pub settings: Settings }
pub struct LoadOutcome { pub store: Store, pub warning: Option<String> }
pub fn load(path: &Path) -> LoadOutcome;      // nunca falha
pub fn save(path: &Path, store: &Store) -> std::io::Result<()>; // escrita atômica (tmp + rename)
pub fn default_path() -> PathBuf;             // ~/Library/Application Support/delux-mouse/profiles.toml
```
`validate` chama os `encode_*` e acumula erros; também: nome não vazio, `scripts[].slot` em `1..18`, sem slot repetido, e o slot de cada script tem `ButtonAction::Key{modifiers:0, usage: hotkey_usage}`.

- [ ] **Step 1: Testes que falham:**
  - `roundtrip`: `save` + `load` devolve `Store` igual, `warning == None`.
  - `missing_file_gives_default`: caminho inexistente → 1 perfil "Padrão", `applied_once == false`, sem warning.
  - `corrupt_file_is_preserved` (Review Focus 3): arquivo com `"{{{"` → store padrão, `warning.is_some()`, existe `profiles.toml.bad` com o conteúdo original.
  - `out_of_range_is_rejected` (Review Focus 3): TOML válido com `dpi = 123` e outro com 9 estágios → mesmo tratamento de corrompido.
  - `changed_sections`: mudar só `polling` → `[Polling]`; mudar DPI e um botão → `[Sensor, Buttons]`; mudar só `scripts[].command` → `[]`.
  - `script_on_slot_0_invalid`.
- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** Commit `feat(profile): profile model and toml store`.

---

### Task 10: Serviço de dispositivo

**Files:** Create `crates/delux-core/{Cargo.toml,src/lib.rs}`

**Consumes:** `Device` (via trait para teste), `Event`, encoders, `Profile`, `Section`.

**Produces:**
```rust
pub trait Link: Send { fn read_event(&self, timeout_ms:i32) -> Result<Option<Event>, HidError>;
                       fn write_frames(&self, f:&[Frame]) -> Result<(), HidError>; }
pub trait Clock: Send + Sync { fn now_ms(&self) -> u64; }
pub enum Connection { NoReceiver, Asleep, Awake }       // Awake = evento há <= 6000 ms
pub struct Status { pub connection: Connection, pub model: Option<u8>, pub battery: Option<(BatteryState,u8)>,
                    pub battery_is_stale: bool, pub dpi_stage: Option<u8> }
pub enum ApplyError { NoReceiver, Asleep, UnsupportedModel(u8), Invalid(Vec<String>),
                      Io{ sent: Vec<Section>, pending: Vec<Section>, message: String } }
pub struct DeviceService { /* … */ }
impl DeviceService {
    pub fn spawn(opener: Box<dyn Fn() -> Result<Box<dyn Link>, HidError> + Send>, clock: Arc<dyn Clock>,
                 on_change: Box<dyn Fn(&Status) + Send>) -> Self;   // thread: lê eventos; sem link, tenta opener a cada 2 s
    pub fn status(&self) -> Status;
    pub fn apply(&self, profile:&Profile, sections:&[Section]) -> Result<(), ApplyError>;
    pub fn factory_reset(&self) -> Result<(), ApplyError>;
}
```
`apply`: valida → exige link → exige modelo `0x30` → exige `Awake` **imediatamente antes de cada seção** → envia na ordem Sensor, Polling, PowerLight, Buttons. Leitura e escrita compartilham o link sob mutex.

- [ ] **Step 1: Testes que falham** (com `FakeLink` que grava frames e `FakeClock`):
  - `battery_goes_stale`: evento de bateria, avançar relógio 6001 ms → `connection == Asleep`, `battery == Some(..)`, `battery_is_stale == true`.
  - `apply_refused_when_asleep` (Review Focus 1): sem evento recente → `Err(Asleep)`, zero frames gravados.
  - `sleep_mid_apply` (Review Focus 1): relógio salta 7000 ms após a 1ª seção → `Err(Io{sent:[Sensor], pending:[Polling,..]})` ou `Err(Asleep)` equivalente carregando as pendentes; frames da 2ª seção não gravados.
  - `unplug_mid_apply` (Review Focus 2): `FakeLink` falha na 2ª escrita → `Io{sent:[Sensor], pending:[Polling,…]}`; depois `opener` volta a funcionar → `status().connection` volta a `Awake` após novo evento.
  - `unsupported_model_blocks_write`: evento com modelo `0x50` → `status().battery` preenchido, `apply` → `Err(UnsupportedModel(0x50))`, zero frames.
  - `apply_order_and_bytes`: perfil padrão, 4 seções → frames gravados == `frames(encode_sensor)`, `frames(encode_polling)`, … nessa ordem.
- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** `delux apply [--profile nome] --yes` na CLI usando `DeviceService`.
- [ ] **Step 6:** Commit `feat(core): device service with awake and model gates`.

---

### Task 11: App Tauri — barra de menu e bateria

**Files:** Create `app/src-tauri/{Cargo.toml,tauri.conf.json,src/main.rs,src/tray.rs}`, `app/ui/index.html` (placeholder)

**Consumes:** `DeviceService`, `Status`, `Store`, `load/save`.

- [ ] **Step 1:** Scaffold Tauri 2 sem bundler de frontend (`frontendDist: "../ui"`), `LSUIElement = true` (sem Dock), identificador `local.edson.delux-mouse`.
- [ ] **Step 2: Teste que falha** em `tray.rs` para função pura:

```rust
pub fn tray_title(s: &Status) -> String;
// NoReceiver → "🖱 —"; Awake+Discharging 84 → "🖱 84%"; Charging 40 → "🖱 ⚡40%";
// Full → "🖱 ⚡100%"; Asleep com última 84 → "🖱 (84%)"; Awake sem bateria ainda → "🖱 …"
pub fn should_notify_low(prev: Option<u8>, now: u8, threshold: u8, charging: bool) -> bool;
// true só ao cruzar de >threshold para <=threshold e !charging; prev None e now<=threshold → true
```
- [ ] **Step 3–4:** implementar → PASS.
- [ ] **Step 5:** `main.rs`: carrega store, `DeviceService::spawn` com `Device::open`; `on_change` atualiza título da bandeja e emite evento Tauri `status`; notificação de bateria baixa (plugin `notification`); menu: perfis (radio; trocar chama `apply` de todas as seções e, em `Err(Asleep)`, marca pendente e notifica "mexa o mouse"), "Abrir ajustes", "Sair"; autostart conforme `Settings`. Fechar a janela só esconde.
- [ ] **Step 6:** `cargo tauri dev` → ícone com porcentagem real aparece; soltar o mouse até dormir → título vira `(NN%)`. Registrar o que foi visto.
- [ ] **Step 7:** Commit `feat(app): menu bar battery`.

---

### Task 12: Janela com abas

**Files:** Create `app/src-tauri/src/commands.rs`; Create `app/ui/{index.html,app.js,style.css}`

**Produces (comandos Tauri, JSON via serde):**
```rust
#[tauri::command] fn get_state() -> UiState;   // { status, store, slot_names: Vec<String>, unverified: Vec<String>, load_warning }
#[tauri::command] fn save_profile(profile: Profile) -> Result<(), Vec<String>>;      // valida, grava, não envia
#[tauri::command] fn apply_profile(name: String) -> Result<ApplyReport, ApplyFailure>; // changed_sections vs último aplicado; 1ª vez = todas
#[tauri::command] fn set_active_profile(name: String) -> Result<ApplyReport, ApplyFailure>;
#[tauri::command] fn create_profile(name: String) / delete_profile(name: String) / factory_reset() / update_settings(s: Settings)
```
`ApplyFailure` = `ApplyError` serializado com `kind` (`asleep`, `no_receiver`, `unsupported_model`, `invalid`, `io`) e mensagem em português.

- [ ] **Step 1: Testes que falham** (Rust, sem UI): `apply_profile` na 1ª vez envia as 4 seções e grava `applied_once = true`; 2ª vez sem mudança envia zero; após `Io{pending}` a próxima tentativa envia só as pendentes; `delete_profile` do ativo ou do último → erro.
- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** UI conforme mockup B (`scratchpad/mockups/.../app-layout.html`, cartão B): barra lateral Sensor / Botões / Scripts / Luz / Energia; cabeçalho com nome do mouse + bateria; seletor de perfil; botão **Aplicar** habilitado só com mudanças. Regras de tela:
  - Faixa fixa enquanto `applied_once == false`: "Valores mostrados são do perfil, não do mouse. Aplicar substitui o que está no mouse."
  - Controles de seções em `unverified` levam selo "não verificado".
  - Modelo ≠ `0x30`: tudo desabilitado + explicação.
  - Erro `asleep`: "Mexa o mouse e tente de novo"; mudanças continuam pendentes.
  - `load_warning` mostrado uma vez.
  - Sensor: 8 linhas de estágio (liga, DPI 50–26000 passo 50, cor), estágio atual (segue evento `DpiStage`), polling, LOD, ripple, angle snap, motion sync. Modo do sensor só se a Task 0 confirmou que existe.
  - Botões: uma linha por botão físico (nomes da Task 7); slot 0 travado.
  - Restaurar fábrica: confirmação explícita.
- [ ] **Step 6:** Verificação manual com o usuário: mudar DPI de um estágio → Aplicar → ele confirma no mouse; fechar e reabrir app → valores persistem.
- [ ] **Step 7:** Commit `feat(app): settings window`.

---

### Task 13: Scripts nos botões

**Depende de:** Task 7 Step 7 ter passado.

**Files:** Create `crates/delux-core/src/scripts.rs`, `app/src-tauri/src/hotkeys.rs`; Modify `commands.rs`, `app/ui/app.js`

**Produces:**
```rust
pub const LOG_CAP: usize = 64 * 1024;
pub struct RunRecord { pub label: String, pub started_ms: u64, pub exit: Option<i32>, pub output: String, pub truncated: bool }
pub struct ScriptRunner { /* … */ }
impl ScriptRunner {
    pub fn trigger(&self, binding: &ScriptBinding) -> bool; // false se o mesmo slot ainda roda; spawn `/bin/zsh -lc <command>` em thread
    pub fn log(&self) -> Vec<RunRecord>;                    // últimas 50
}
pub fn next_free_usage(used: &[u8]) -> Option<u8>;          // F13..F24 = 0x68..=0x73
#[tauri::command] fn bind_script(profile:String, slot:usize, command:String, label:String) -> Result<(), String>;
#[tauri::command] fn unbind_script(profile:String, slot:usize) -> Result<(), String>; // restaura ação de fábrica do slot
#[tauri::command] fn script_log() -> Vec<RunRecord>;
```
`bind_script`: escolhe `next_free_usage`, **registra o atalho global primeiro**; só se registrar, altera `buttons[slot]` para `Key{0,usage}`, adiciona `ScriptBinding`, grava e aplica seção Buttons. Ao trocar de perfil ativo: desregistrar atalhos do antigo, registrar os do novo.

- [ ] **Step 1: Testes que falham:**
  - `runs_and_captures`: comando `echo oi` → registro com `exit == Some(0)`, `output == "oi\n"`.
  - `output_is_capped` (Review Focus 4): `yes | head -c 200000` → `output.len() <= LOG_CAP`, `truncated == true`.
  - `hung_script_does_not_block` (Review Focus 4): `sleep 30` → `trigger` retorna em < 100 ms; segundo `trigger` do mesmo slot → `false`; slot diferente → `true`.
  - `next_free_usage`: `[]` → `0x68`; todos os 12 usados → `None`.
  - `hotkey_conflict_leaves_profile_untouched` (Review Focus 5): com registrador falso que falha → `bind_script` devolve `Err`, perfil em disco idêntico, zero frames enviados.
  - `slot_0_rejected`.
- [ ] **Step 2–4:** FAIL → implementar → PASS.
- [ ] **Step 5:** Aba Scripts: lista botão → rótulo + comando, adicionar/remover, aviso "o botão perde a função original enquanto atribuído; só funciona com o app aberto", log das execuções.
- [ ] **Step 6:** Verificação manual com o usuário: atribuir um botão lateral a `open -a Calculator` → clique abre a Calculadora; remover → botão volta a ser voltar/avançar.
- [ ] **Step 7:** Commit `feat(app): button-triggered scripts`.

---

### Task 14: Empacotar e roteiro final

**Files:** Create `README.md`; Modify `app/src-tauri/tauri.conf.json`

- [ ] **Step 1:** `cargo tauri build` → `Delux Mouse.app` (sem assinatura); copiar para `~/Applications`.
- [ ] **Step 2:** `cargo test --workspace` → tudo PASS; `cargo clippy --workspace -- -D warnings` limpo.
- [ ] **Step 3:** Roteiro manual com o usuário no `.app` instalado: abrir → bateria na barra; reiniciar sessão → abre sozinho; tirar e recolocar o receptor → reconecta sem reabrir; mouse dormindo + Aplicar → pede para mexer; trocar perfil pelo menu → mouse muda; bateria: conferir com o que o mouse indica.
- [ ] **Step 4:** `README.md`: o que faz, como compilar, limites (sem leitura do mouse, sem macros, só receptor 8K), como recuperar (Restaurar fábrica; driver Windows), como remover.
- [ ] **Step 5:** Atualizar o design: lista do que ficou "não verificado". Commit `chore: package app and document`.

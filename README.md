# Delux Mouse

App de barra de menu para macOS que substitui o driver Windows do **Delux M900 PRO 8K** (PAW3395, receptor 2.4G 8K `1D57:FA65`).

- Bateria do mouse sempre visível na barra de menu, com aviso de carga baixa.
- Janela de ajustes: estágios de DPI e cores, polling, opções do sensor, função dos botões, luz, sleep e debounce.
- Botões do mouse podem rodar comandos/scripts no Mac.
- Vários perfis; troca pelo menu.

Uso pessoal, não assinado. Protocolo obtido por análise estática do driver oficial — ver `docs/protocol/delux-protocol.md`.

## Limites

- **O mouse não informa sua configuração.** O app guarda perfis em
  `~/Library/Application Support/delux-mouse/profiles.toml` e só envia. O que aparece na tela é o perfil, não uma leitura do mouse.
- **O primeiro "Aplicar" substitui o que estava no mouse.**
- Só o receptor 8K e o modelo de código `0x30`. Sem modo cabo, sem dongle antigo, sem outros modelos (a bateria aparece, a escrita é bloqueada).
- Sem macros gravadas no mouse e sem atualização de firmware.
- Comandos só são enviados com o mouse acordado (sinal nos últimos 6 s); dormindo, o ajuste fica pendente.
- Scripts só funcionam com o app aberto; o botão atribuído perde a função original (vira F13–F24).

## O que foi confirmado no hardware (2026-11-10)

Confirmado: DPI/estágios, luz, mapa de botões, tecla F13 emitida pelo botão, bateria e estado "carregando".

**Não verificado** (enviado sem erro, efeito não confirmado; marcado na tela): polling rate, LOD, ripple, angle snapping, motion sync, sleep, debounce.

## Recuperação

- Aba Energia → **Restaurar padrão de fábrica**: envia o reset e reenvia todos os padrões do driver (DPI 400/800/1200/1600/3200/5000, botões originais).
- Pela linha de comando: `delux reset-buttons --yes`, `delux set-dpi 400,800,1600 --yes`.
- Em último caso, o driver oficial no Windows regrava tudo.
- O mouse funciona como mouse comum independentemente do app.

## Compilar

```bash
cargo test --workspace
```

```bash
cargo install tauri-cli --locked --version "^2"
```

```bash
cd app/src-tauri && cargo tauri build
```

Resultado: `target/release/bundle/macos/Delux Mouse.app`.

CLI de diagnóstico (nada é enviado sem `--yes`):

```bash
cargo run -p delux-cli -- listen --secs 10
```

## Estrutura

| pasta | papel |
|---|---|
| `crates/delux-proto` | pacotes e eventos do protocolo, sem I/O |
| `crates/delux-hid` | transporte `hidapi` |
| `crates/delux-profile` | perfis, validação, TOML |
| `crates/delux-core` | serviço do dispositivo (travas de envio), lógica do app, scripts |
| `crates/delux-cli` | `delux`, diagnóstico |
| `app/` | app Tauri (barra de menu + janela) |

## Remover

Sair pelo menu, apagar `~/Applications/Delux Mouse.app`, a pasta `~/Library/Application Support/delux-mouse` e o item de login em `~/Library/LaunchAgents` (nome contém `Delux Mouse`).

# Delux M900 PRO para macOS — Design

Data: 2026-11-09. Status: aprovado pelo usuário em 2026-11-09.

Legenda: **[usuário]** = dito ou escolhido por ele. **[decisão minha]** = escolha do autor do design, pode ser vetada. **[medido]** = observado no hardware dele. **[do driver]** = extraído por análise estática do driver Windows, não testado no hardware.

## 1. Intenção e porquê

O mouse Delux M900 PRO 8K (sensor PAW3395) só tem software de configuração para Windows. No Mac, o usuário não consegue ver a bateria nem configurar o mouse, e sente falta de disparar scripts pelos botões enquanto trabalha. **[usuário]**

O app substitui o driver Windows no Mac.

## 2. Objetivos

1. Mostrar a bateria do mouse o tempo todo na barra de menu. **[usuário]**
2. Ajustar todas as funções do mouse que o driver Windows ajusta. **[usuário: "todas as funções"]**
3. Fazer botões do mouse dispararem scripts/comandos no Mac. **[usuário]**
4. Formato: ícone na barra de menu + janela completa com abas. **[o usuário, escolheu mockup B]**

## 3. Fora de escopo (v1)

- Macros gravadas no firmware (comando `0x09`): formato só parcialmente decodificado. **[decisão minha]**
- Modo cabo (PIDs por modelo, feature reports) e dongle antigo `FA60`. v1 só fala com o receptor 8K `1D57:FA65`. **[decisão minha]**
- Outros modelos Delux (variantes `0x3F`, `0x50` e as demais). O código não deve impedir adicioná-los depois, mas nada é testado. **[decisão minha]**
- Atualização de firmware. Nunca. **[decisão minha]**
- Distribuição pública, assinatura/notarização. App é de uso pessoal. **[decisão minha]**

## 4. Anti-objetivos (falha mesmo se "funcionar")

- Deixar o mouse em estado inutilizável ou que exija Windows para recuperar.
- Enviar ao mouse pacote cujo layout não foi confirmado, sem o usuário saber que é um teste.
- Exigir driver de kernel / extensão de sistema.
- Mostrar valores como se fossem lidos do mouse quando são só o perfil local.

## 5. Fatos do hardware

Spec completa do protocolo: `docs/protocol/delux-protocol.md`. Resumo do que importa:

- Receptor USB `2.4G 8K HS Receiver`, VID `0x1D57`, PID `0xFA65`. **[medido]**
- Uma interface HID (iface 2) carrega, no mesmo dispositivo macOS: comandos (usage page `0xFF00`, report ID 4, 63 bytes de saída) e eventos (report ID 3). Abre via `hidapi` sem pedir permissão do macOS. **[medido]**
- Evento = `03, modelo, tipo, v0, v1`. **[medido]**
  - Modelo do usuário: `0x30` (página MS_4 do driver).
  - Bateria: tipo `0x40`, a cada 2 s com mouse acordado. Visto `03 30 40 01 54` = 84 %, v0 = 1. Mouse dormindo = silêncio total.
  - Troca de estágio DPI: tipo `0x10`, v0 = estágio 1..6 (6 estágios ativos hoje).
- Significado de v0 na bateria (`3` = carregando, `2` = carga completa) **[do driver, de outra variante — confirmar no hardware]**.
- Enquadramento do comando: `04, N+5, 00, pacote[N], checksum BE (soma 16-bit dos bytes anteriores), zeros`. **[do driver]**
- Comandos: `0x04` DPI/sensor, `0x05` luz/energia/debounce, `0x06` polling, `0x08` mapa de botões (18 slots), `0x0C` reset de fábrica. **[do driver]**
- **O mouse não devolve configuração.** O driver Windows nunca lê; só envia. Sem ack no caminho FA65. **[do driver]**
- Ordem dos campos do `0x04` na variante `0x30` é inferida. **[do driver, não confirmado]**
- Polling acima de 1000 Hz só com modo do sensor "Gaming". **[do driver]**

## 6. Como funciona

### 6.1 Peças

- **Núcleo de protocolo** (crate Rust, sem UI): monta/valida pacotes, decodifica eventos. Puro, testável sem hardware. Vetores de teste vêm da spec (ex.: polling 8000 Hz = `04 0E 00 06 09 01 74 8B 00 00 00 00 01 21`).
- **Transporte**: `hidapi`; acha `1D57:FA65`, abre a interface com usage page `0xFF00`; uma thread lê eventos, escritas serializadas com as pausas do driver (200 ms por pedaço, 500 ms após comando). Reconecta sozinho se o receptor sair/voltar.
- **Perfil**: arquivo local (TOML em `~/Library/Application Support/`) com tudo que foi configurado. É a única fonte de "estado atual", já que o mouse não devolve nada.
- **App**: barra de menu + janela.
- **CLI de diagnóstico** embutida (`listen`, `send-raw`), usada na verificação em hardware (seção 8).

Tecnologia: Rust; UI com Tauri 2 (núcleo Rust, tela em HTML/CSS/JS simples, sem framework). **[decisão minha]** Motivo: bandeja, janela, iniciar no login e empacotar `.app` já resolvidos. Alternativa pura-Rust (egui) é aceitável se o o usuário preferir.

### 6.2 Barra de menu

- Ícone + porcentagem. Estados: porcentagem; carregando; "dormindo/desconectado" (sem evento há > 6 s — mostra último valor conhecido esmaecido, nunca um número como se fosse atual).
- Notificação de bateria baixa (limiar padrão 20 %, ajustável). **[decisão minha]**
- Menu: perfil ativo, troca rápida de perfil, abrir janela, sair.
- Sem ícone no Dock; inicia no login (opcional, ligado por padrão). **[decisão minha]**

### 6.3 Janela — abas

| Aba | Conteúdo | Comando |
|---|---|---|
| Sensor | 8 estágios de DPI (liga/desliga, 50–26000 passo 50, cor), estágio atual, polling 125–8000 Hz, LOD 1/2 mm, ripple, angle snap, motion sync, modo do sensor | `0x04`, `0x06` |
| Botões | função de cada botão no firmware: clique, DPI, tecla+modificador, mídia, desligado | `0x08` |
| Scripts | botão → comando shell / arquivo de script / atalho de teclado | ver 6.4 |
| Luz | modo, brilho, velocidade, cor, timer de apagar | `0x05` |
| Energia | tempo de sleep, debounce | `0x05` |

- Mudanças ficam pendentes até **Aplicar**; Aplicar envia só as seções alteradas.
- Enquanto um perfil nunca foi aplicado nesta instalação, a janela avisa: "valores mostrados são do perfil, não do mouse". Primeiro perfil é criado com os padrões do driver.
- Vários perfis nomeados (ex.: Trabalho, Jogo). **[decisão minha, inspirado no mockup]** Trocar de perfil pelo menu aplica na hora (alguns segundos; exige mouse acordado, senão fica pendente com aviso). Atribuições de script fazem parte do perfil.
- Macros fora da v1: o usuário concordou ao revisar o resumo. **[usuário]**
- Polling > 1000 Hz força modo "Gaming" com aviso (mesma regra do driver).
- "Restaurar padrão de fábrica" com confirmação.
- Eventos do mouse atualizam a UI (estágio de DPI trocado no botão físico).

### 6.4 Botões → scripts

Mecanismo **[decisão minha]**: o botão escolhido é mapeado no firmware para uma tecla que nenhum teclado tem (F13–F24, com modificadores se faltar). O app registra essa tecla como atalho global e roda o comando associado.

- Não precisa de permissão de Acessibilidade nem interceptar o mouse.
- Só funciona com o app rodando; sem o app, o botão manda uma tecla inerte. A UI diz isso.
- Comando roda via `/bin/zsh -lc` como o usuário, sem bloquear a UI; saída/erro vão para um log visível na aba Scripts.
- Clique esquerdo não pode ser atribuído a script (proteção contra travar o mouse).

## 7. Tratamento de erro

- Receptor ausente: app segue rodando, ícone "desconectado", tenta de novo.
- Mouse dormindo ao Aplicar: sem ack não dá para saber se chegou. O app exige evento recebido nos últimos 6 s antes de enviar; senão pede "mexa o mouse" e espera.
- Falha de escrita HID: mostra erro, mantém mudanças como pendentes.
- Evento com modelo ≠ `0x30`: app mostra bateria, mas bloqueia escrita e explica que o modelo não é suportado.

## 8. Verificação no hardware (obrigatória, em ordem)

Cada passo é um teste com o o usuário presente, um comando por vez, do mais inofensivo ao mais arriscado. Nada passa para a UI antes de confirmado.

Quem executa: o agente construtor roda os comandos (`send-raw`) no Mac do usuário, com ele ao lado do mouse relatando o que vê. **[decisão minha]** Seção que não der para confirmar por observação (ex.: debounce) entra na UI marcada "não verificado"; seção que se comportar diferente do esperado fica fora até ser reanalisada.

0. Antes de qualquer envio: reanalisar no driver a função `FUN_004d0230` (variante `0x30`) para fechar (a) ordem dos campos do `0x04`, (b) se existe campo "modo do sensor" nessa variante — na spec atual só a variante `0x50` tem — e (c) o byte 49 (`0x03`). Se a variante `0x30` não tiver modo do sensor: a aba Sensor não mostra esse controle e a regra "polling > 1000 Hz exige Gaming" não se aplica.
1. Escuta (feito): modelo e bateria.
2. `0x06` polling 1000 Hz (valor provavelmente já em uso) → conferir que nada muda; depois 500 Hz → conferir evento `0x20` e/ou medição.
3. `0x04` DPI: confirma ordem dos campos da variante `0x30`. Mandar perfil com estágios bem distintos e conferir pelo movimento e pelo evento `0x10`. Se o comportamento não bater com o layout inferido, parar e reanalisar o driver (função `FUN_004d0230`).
4. `0x05` luz/sleep/debounce.
5. `0x08` botões: primeiro só trocar botão lateral; nunca remapear clique esquerdo em teste.
5b. Tecla inerte: mapear um botão lateral para F13 (`11 00 68`) e conferir que o macOS entrega a tecla e que o atalho global registra. Se falhar: plano B para scripts é interceptar o botão do mouse com event tap (exige permissão de Acessibilidade); voltar ao usuário antes de seguir por esse caminho.
6. Bateria: plugar cabo e conferir v0 (carregando / completo).
7. `0x0C` reset de fábrica: testar por último, de propósito, para saber que a saída de emergência funciona.

Rede de segurança: o driver Windows oficial consegue reescrever tudo; reset `0x0C`; o mouse funciona como mouse comum independentemente da configuração.

## 9. Testes automáticos

- Núcleo de protocolo: testes unitários de codificação (bytes exatos, checksum, divisão em pedaços) e decodificação de eventos, incluindo os eventos reais capturados.
- Perfil: ida e volta de serialização; validação de limites (DPI múltiplo de 50, 1–8 estágios, etc.).
- Transporte e UI: testados manualmente pelo roteiro da seção 8.

## 10. Deixado para quem constrói

- Layout interno de crates/módulos, formato exato do TOML, visual da janela (seguir o mockup B em espírito), ícone.
- Tabela completa de códigos de botão: usar a da spec; códigos sem nome ficam fora da UI.
- Se os passos 3–5 da seção 8 revelarem layout diferente do inferido: corrigir a spec e o núcleo; isso é esperado, não mudança de escopo.

## 10b. Duas perguntas que ficaram sem resposta

o usuário aprovou o documento sem responder a estas; valem os padrões abaixo. **[decisão minha]**

- **Primeiro Aplicar sobrescreve a configuração atual do mouse.** Padrão: aceito. O primeiro perfil nasce com valores padrão editáveis, e a janela mostra um aviso explícito antes do primeiro Aplicar ("isto substitui o que está no mouse; ajuste DPI/polling antes se quiser manter os seus").
- **Quais botões vão para scripts.** Padrão: nenhum pré-atribuído. A aba Scripts deixa escolher qualquer botão exceto clique esquerdo e avisa que o botão perde a função original enquanto estiver atribuído. A correspondência botão físico ↔ slot do firmware é levantada no passo 5 da seção 8.

## 11. Local

Projeto em `~/delux-mouse` (git novo). **[decisão minha]**

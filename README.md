# flamenote

Notas flutuantes a partir do texto selecionado, no espírito do Flameshot.

Ao iniciar, o programa fica na bandeja. Um atalho global arma a seleção: o cursor vira uma cruz e a caixa de ferramentas ainda não aparece. Esc nesse momento só devolve o cursor. Depois de selecionar texto na janela em foco, a caixa surge ao lado dessa seleção. A caixa tem duas ferramentas fixas: a primeira coloca o texto num bilhete branco, de cantos arredondados, sempre por cima das outras janelas; a segunda grava o texto num arquivo `.txt` na pasta de destino e deixa esse arquivo na área de transferência, pronto para colar. Consultas configuradas acrescentam até três botões que enviam o texto, com um pré-prompt, a um servidor compatível com OpenAI. O bilhete pode ser arrastado. Um clique duplo fecha o bilhete. Esc, com a caixa já aberta, fecha só a caixa.

## Requisitos

Esta versão fala com o X11. No Wayland ela não sobe.

A bandeja usa o protocolo clássico do systray (XEmbed), o mesmo da i3bar com `tray_output`. Sem bandeja, o atalho continua valendo e o programa avisa isso uma vez.

GTK 3 desenha as janelas. Com um compositor (picom, por exemplo) a sombra em volta do cartão aparece. Sem compositor, o flamenote recorta os cantos com uma máscara, para não deixar um quadrado preto.

## Compilar e executar

```bash
cargo build --release
./target/release/flamenote
```

No i3, uma linha de autostart:

```
exec --no-startup-id flamenote
```

A segunda execução, enquanto a primeira vive, arma a seleção em vez de criar outro processo. Se a trava de instância já estiver ocupada e o pedido não chegar ao processo principal, o programa escreve `flamenote já está em execução.` e sai.

## Configuração

O arquivo fica em `$XDG_CONFIG_HOME/flamenote/config.toml`, ou em `~/.config/flamenote/config.toml`. `FLAMENOTE_CONFIG` aponta para outro caminho. Na primeira execução o programa grava o padrão, se o arquivo ainda não existir, e relê a cada 2 segundos.

```toml
# Atalho global do flamenote.
# Modificadores: Ctrl, Alt, Shift, Super.
# Exemplos: "Super+Shift+N", "Ctrl+Alt+N", "F9"
hotkey = "Super+Shift+N"
# Pasta onde o texto selecionado é salvo como arquivo.
# Aceita caminho absoluto ou começando com ~.
copies_dir = "~/copias"

[llm]
base_url = "http://localhost:11434"
model = "qwen3:14b"
api_key = ""

[[queries]]
prompt = "Resuma em uma frase:"
destination = "clipboard"

[[queries]]
prompt = "Liste as tarefas:"
destination = "file"
```

O atalho padrão é `Super+Shift+N`. Esc não pode ser o atalho. Durante a seleção, Esc só devolve o cursor. Com a caixa aberta, Esc fecha a caixa. Um atalho inválido volta para o padrão e a mensagem sai no terminal.

A pasta de destino padrão é `~/copias`. Ela é criada quando o primeiro arquivo é salvo. Caminhos relativos são resolvidos a partir da pasta pessoal. Alterações no arquivo são relidas a cada 2 segundos.

`[llm]` aceita qualquer servidor com o endpoint `/v1/chat/completions` no formato OpenAI. `base_url` pode terminar em `/v1` ou já incluir o caminho completo; sem sufixo, o flamenote acrescenta `/v1/chat/completions`. Deixe `api_key` vazia em servidores locais, como o Ollama. Até três `[[queries]]` podem ser definidas. O `prompt` é enviado como mensagem de sistema e o texto selecionado como mensagem do usuário. `destination = "file"` grava a resposta na pasta de destino e deixa o arquivo na área de transferência; `destination = "clipboard"` copia o texto da resposta. O limite de saída é de 4096 tokens; uma resposta cortada pelo servidor não é salva e gera uma notificação de truncamento. O arquivo de configuração guarda a chave de API em texto puro, mas é gravado com permissão `600`.

O clique direito na bandeja abre “Configurações”. O campo do atalho, ao ser clicado, captura a próxima combinação. Ela é gravada na hora nesse arquivo e passa a valer em seguida. O campo “Pasta de destino” grava a pasta ao pressionar Enter ou ao sair do campo; deixar vazio volta para `~/copias`. Na aba “LLM”, o botão “Salvar configuração LLM” grava a URL, o modelo, a chave e as três consultas. Consultas com prompt vazio ficam desativadas.

`FLAMENOTE_DEBUG=1` escreve no terminal o que a thread do X11 está fazendo.

## O que esta versão faz

- O atalho, o clique esquerdo na bandeja e o item “Selecionar texto” do menu armam a seleção. O cursor do sistema some e uma cruz segue o ponteiro. A cruz não recebe cliques, então o arrasto continua valendo na janela que está embaixo. A caixa não abre nesse momento. Texto que já estava selecionado antes do atalho não abre a caixa.
- Esc com a cruz na tela devolve o cursor e não fecha bilhetes. O atalho de novo também cancela.
- Depois de um arrasto que seleciona texto, ou de uma seleção nova na janela em foco, a caixa aparece ao lado do fim da seleção. As ferramentas começam habilitadas com esse texto. Elas ficam desabilitadas se a seleção sumir antes do clique. Espaço em branco conta como nada selecionado.
- As ferramentas leem a seleção primária do X11 (PRIMARY). O dono dessa seleção precisa ser a janela em foco, um parente dela na árvore de janelas, outro cliente do mesmo processo (`_NET_WM_PID`) ou a mesma conexão X11. Isso pega a janela invisível em que o GTK e o Chromium guardam a seleção. Duas janelas do mesmo processo podem habilitar as ferramentas mesmo com a seleção na que não está em foco.
- A caixa de ferramentas não rouba o foco. Clique fora dela não a fecha. Fecham: Esc, o atalho de novo, o clique esquerdo na bandeja, ou o uso de uma ferramenta.
- O clique direito na bandeja mostra “Selecionar texto”, “Configurações” e “Sair”.
- O bilhete mostra o texto capturado, até 20 mil caracteres. Dá para arrastar. Um deslocamento menor que uns 3 pixels não conta como arrasto, para o clique duplo continuar fechando o bilhete.
- O botão de arquivo usa até 30 caracteres da seleção como nome, trocando espaços por `_` e descartando outros caracteres. O texto é gravado com `.txt` na pasta de destino e o arquivo é copiado como item colável (`x-special/gnome-copied-files`), com uma notificação. Um nome repetido sobrescreve o arquivo anterior.
- Cada consulta configurada ganha um botão com ícone de conversa; o tooltip mostra o pré-prompt. Ao clicar, a caixa fecha, o cursor fica livre e a requisição corre em segundo plano. Ao terminar, a resposta é gravada como arquivo ou copiada como texto, conforme o checkbox da consulta, e uma notificação informa o resultado ou o erro.
- A roda do mouse rola o texto quando ele não cabe.

## Licença

MIT. Copyright (c) 2026 EduMelo.

* reconnect logic / error-handling in the chatbot task (should quit so we can see it)
* drop nom for now, use clap
* chatHist class with getters for sincetime, filter by nick, etc. plugins specify how many messages of context they want when they register
* ai summaries of chat, eg --since offset. Ideally do as a plugin, but again wasm needs to be able to make HTTP calls
* change wasm runtime to WasmEdge
  * change should be isolated to 1 or 2 files, if not, refactor
  * plugins to make http reqs using one of: https://github.com/vasilev/HTTP-request-from-inside-WASM?tab=readme-ov-file#rust-wasi
* status grpc endpoint, that lists servers & channels connected to, #people in them, etc
* plugins
  * strip tracking (basically any query component) from pasted links
  * spotify link -> artist, track. Needs the ability to http outbound

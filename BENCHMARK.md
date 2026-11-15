# Medición inicial

Equipo: detalles identificadores omitidos. Compilación `cargo build --release`. Composición [`examples/title-card.html`](examples/title-card.html): 4 s, 24 fps, 96 frames, salida a 1920×1080 (`--scale 3`). Medición de tiempo real y RSS con herramientas del sistema.

| Prueba | Tiempo interno | Tiempo real | RSS máximo | Resultado |
| --- | ---: | ---: | ---: | --- |
| `bench` (solo dibujo) | 0,481 s | 0,55 s | 25.849.856 bytes | 28 frames dibujados, 68 reutilizados |
| `render` MP4 nativo, 2 hilos | 2,021 s | 2,12 s | 50.044.928 bytes | H.264 MP4 de 366 KiB |

Binario final: 2.907.536 bytes. El MP4 se abrió y transcodificó con una herramienta externa. `cargo test` y `cargo clippy --all-targets -- -D warnings` pasaron.

Esto prueba una composición pequeña, dominada por escenas estáticas. No mide el rendimiento en una PC antigua ni efectos CSS/GSAP que el backend nativo todavía no soporta. No se deben extrapolar estos resultados a otras composiciones o equipos. Para comparar motores, usar una composición representativa y medir tiempo total, RSS, calidad visual y tamaño de salida en el mismo hardware.

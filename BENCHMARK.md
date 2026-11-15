# Medición inicial

Equipo: detalles identificadores omitidos. Compilación `cargo build --release`. Composición [`examples/title-card.html`](examples/title-card.html): 4 s, 24 fps, 96 frames, salida a 1920×1080 (`--scale 3`). Medición de tiempo real y RSS con herramientas del sistema.

| Prueba | Tiempo interno | Tiempo real | RSS máximo | Resultado |
| --- | ---: | ---: | ---: | --- |
| `bench` (solo dibujo) | 0,481 s | 0,55 s | 25.849.856 bytes | 28 frames dibujados, 68 reutilizados |
| `render` MP4 nativo, 2 hilos | 2,021 s | 2,12 s | 50.044.928 bytes | H.264 MP4 de 366 KiB |

Binario final: 2.907.536 bytes. El MP4 se abrió y transcodificó con una herramienta externa. `cargo test` y `cargo clippy --all-targets -- -D warnings` pasaron.

Esto prueba una composición pequeña, dominada por escenas estáticas. No mide el rendimiento en una PC antigua ni efectos CSS/GSAP que el backend nativo todavía no soporta. No se deben extrapolar estos resultados a otras composiciones o equipos. Para comparar motores, usar una composición representativa y medir tiempo total, RSS, calidad visual y tamaño de salida en el mismo hardware.

## Comparación del core nativo

Mismo equipo y composición. Binarios de release del commit base y de esta revisión. Mediana de cinco ejecuciones por versión, alternadas, a 1920×1080 con 24 fps y 96 fotogramas. Se usó el tiempo interno de la CLI. El MP4 se codificó con OpenH264 y dos hilos.

| Prueba | Antes | Ahora | Reducción de tiempo |
| --- | ---: | ---: | ---: |
| `bench` (solo dibujo) | 0,465 s | 0,099 s | 79 % |
| `render` (MP4 nativo) | 1,876 s | 1,445 s | 23 % |

Los diez MP4 tuvieron el mismo SHA-256. Esta comparación mide una composición con fades y pausas estáticas. Los resultados dependen de la composición y del equipo.

## MP4 nativo a 60 fps

Misma configuración de prueba. Composición de tres segundos con texto, bloques sólidos, desplazamientos y opacidad, anterior al degradado del ejemplo. Para medir a 4K con ambas versiones se usó un lienzo de entrada de 960×540 y salida de 3840×2160. Para 1080p se usó la entrada de 640×360. Medianas de tres ejecuciones de release por versión, con 180 fotogramas y `--threads 4`.

| Salida | Antes, dibujo | Ahora, dibujo | Antes, MP4 | Ahora, MP4 |
| --- | ---: | ---: | ---: | ---: |
| 1920×1080 | 0,214 s | 0,094 s | 2,277 s | 1,908 s |
| 3840×2160 | 0,767 s | 0,171 s | 7,127 s | 6,131 s |

Las pruebas verifican la cadencia del MP4 a 30 y 60 fps en ambas resoluciones. La exportación 4K se ejecutó por debajo de 60 fotogramas por segundo. El ejemplo con degradado y esquinas redondeadas también se exportó en 4K a 60 fps y se decodificó tras la exportación.

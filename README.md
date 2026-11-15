# Aether

Aether es una herramienta local en Rust para generar fotogramas PNG y videos MP4 H.264 a partir de composiciones HTML con tiempos. El backend nativo admite el perfil descrito abajo, dibuja sin iniciar un navegador ni Node.js y devuelve errores explícitos para las funciones no soportadas. El codificador MP4 nativo integra OpenH264; FFmpeg es opcional. Licencia Apache 2.0.

## Empezar

Se necesita Rust estable para compilar. La ruta MP4 nativa incluye OpenH264 y no requiere instalar FFmpeg. FFmpeg queda disponible como codificador opcional con `--encoder ffmpeg`.

```bash
cargo build --release
cargo run --release -- inspect examples/title-card.html
cargo run --release -- render examples/title-card.html --output /tmp/title.png --frame 24 --fps 24 --scale 0.5
cargo run --release -- render examples/title-card.html --output /tmp/title.mp4 --fps 24 --scale 0.5 --threads 2
# Opcional, si ya tienes FFmpeg:
cargo run --release -- render examples/title-card.html --output /tmp/title-ffmpeg.mp4 --encoder ffmpeg
```

`--scale 0.5` reduce ancho y alto a la mitad, de modo que cada fotograma contiene una cuarta parte de los píxeles. `--threads` limita los hilos de codificación. Ambos codificadores producen H.264/yuv420p; el modo FFmpeg usa `veryfast` y CRF 25. El renderizador mantiene un solo fotograma RGBA y hasta 64 MiB de imágenes ya escaladas; las escenas visualmente idénticas reutilizan el fotograma anterior.

## Entradas compatibles

| Entrada | Estado |
| --- | --- |
| `data-composition-id`, `data-width`, `data-height`, `data-duration` | Sí; en el elemento raíz o en `<meta>` |
| `data-start` numérico o referencia a otro `id`, con `+/-` offset | Sí |
| `data-track-index`, clips con `<div>`, texto e imágenes locales PNG/JPEG/WebP | Sí |
| Selectores CSS de tipo, clase, ID y descendientes; estilo inline | Sí |
| `left`, `top`, `width`, `height` en px o %, colores hex/nombres básicos, opacidad, fuente, centrado flex, `object-fit` | Sí |
| `data-hf-fade-in`, `data-hf-fade-out` | Extensión nativa; segundos, transición lineal |
| GSAP, `<script>`, `<video>`, `<audio>`, `<svg>`, canvas, efectos complejos, composición externa, CSS general | Aún no; error explícito |

Las fuentes se buscan en el sistema. El texto admite una línea y no implementa todo el modelado tipográfico del navegador. Solo se aceptan imágenes locales. Para una composición real, ejecuta primero `inspect`; si una función no está cubierta, el programa explica la primera incompatibilidad.

## Medir en tu equipo

```bash
cargo run --release -- bench examples/title-card.html --fps 24 --scale 0.5
```

`bench` mide el dibujo en memoria, **sin codificación MP4**. La salida muestra el tiempo y los fotogramas rasterizados y reutilizados. Para medir el pipeline completo, usa `render` hacia MP4. Para comparar motores, renderiza la misma composición compatible con idéntica resolución, FPS y opciones de codificación en el mismo equipo. Registra el tiempo total, la memoria máxima y el tamaño de salida; no extrapoles resultados de una máquina a otra.

[Medición inicial](BENCHMARK.md).

El código usa APIs portables de Rust y el repositorio incluye una matriz de CI para Linux, macOS y Windows.

## Próximos pasos

1. Añadir más propiedades CSS y casos de layout a partir de composiciones reales, con fixtures visuales.
2. Medir el pipeline completo de MP4 en Windows y Linux, además de macOS, y perfilar CPU y memoria por etapa.
3. Evaluar la calidad y el tamaño de OpenH264 frente a FFmpeg en escenas reales.
4. Ofrecer un backend de compatibilidad para HTML/JS general cuando el perfil nativo no alcance.

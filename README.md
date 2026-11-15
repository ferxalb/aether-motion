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

`--scale 0.5` reduce ancho y alto a la mitad, de modo que cada fotograma contiene una cuarta parte de los píxeles. En exportaciones nativas de al menos 1080p, `--threads 2` o más permite solapar dibujo y conversión de color con la codificación mediante una cola de un fotograma. Ambos codificadores producen H.264/yuv420p; el modo FFmpeg usa `veryfast` y CRF 25. El renderizador mantiene un solo fotograma RGBA, hasta 64 MiB de imágenes escaladas y hasta 32 MiB de máscaras de texto. Cuando el estado visible coincide con el fotograma anterior, reutiliza el RGBA y la conversión YUV del codificador nativo.

## Entradas compatibles

| Entrada | Estado |
| --- | --- |
| `data-composition-id`, `data-width`, `data-height`, `data-duration` | Sí; en el elemento raíz o en `<meta>` |
| `data-start` numérico o referencia a otro `id`, con `+/-` offset | Sí |
| `data-track-index`, clips con `<div>`, texto e imágenes locales PNG/JPEG/WebP | Sí |
| Selectores CSS de tipo, clase, ID y descendientes; estilo inline | Sí |
| `left`, `top`, `width`, `height` en px o %, colores hex/nombres básicos, opacidad, fuente, centrado flex, `object-fit` | Sí |
| `border-radius` y `linear-gradient(...)` | Radio de fondo en px y gradientes verticales u horizontales de dos colores opacos |
| `data-hf-fade-in`, `data-hf-fade-out` | Extensión nativa; segundos, transición lineal |
| `data-hf-keyframes` | Extensión nativa; posición y opacidad interpoladas con cuatro curvas |
| GSAP, `<script>`, `<video>`, `<audio>`, `<svg>`, canvas, efectos complejos, composición externa, CSS general | Aún no; error explícito |

Las fuentes se buscan en el sistema. El texto admite una línea y no implementa todo el modelado tipográfico del navegador. Solo se aceptan imágenes locales. Para una composición real, ejecuta primero `inspect`; si una función no está cubierta, el programa explica la primera incompatibilidad.

## Exportar a 1080p y 4K

```bash
aether-hyperframes render examples/motion.html --size 1080p --fps 60 --output /tmp/motion-1080p.mp4
aether-hyperframes render examples/motion.html --size 4k --fps 30 --output /tmp/motion-4k.mp4
```

Los presets producen 1920×1080 y 3840×2160 a partir de composiciones 16:9. Ambos admiten 30 y 60 fps. Para otras proporciones usa `--scale` o define las dimensiones de salida en el HTML. El MP4 nativo admite hasta 60 fps y dimensiones pares hasta 3840×2160 o 2160×3840. La cadencia del archivo es constante. El tiempo de exportación depende de la escena y del equipo.

En MCP, los mismos presets están disponibles mediante `size`. Usa `size` o `scale` en cada llamada.

## Crear y revisar animaciones

[La composición de ejemplo](examples/motion.html) combina entrada y salida con movimiento, capas y tiempos relativos. Los fotogramas clave usan segundos locales al clip. El atributo admite `x`, `y`, `opacity` y `easing`. Consulta el contrato completo en [AGENTS.md](AGENTS.md).

```bash
aether-hyperframes check examples/motion.html
aether-hyperframes render examples/motion.html --output /tmp/motion.png --fps 30 --frame 12
aether-hyperframes render examples/motion.html --output /tmp/motion.mp4 --fps 30
```

Para agentes, `aether-hyperframes mcp` expone el contrato en `aether_guide`. `aether_check` devuelve tiempos resueltos y tiempos locales de los fotogramas clave en `animations`, con un error y una corrección sugerida cuando falla. `aether_snapshot` devuelve un PNG con el estado en el segundo solicitado.

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

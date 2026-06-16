# Adobo Reader 🚀
### Visualizador de PDF Minimalista y Ultra-rápido Acelerado por GPU en Rust

Adobo Reader es un lector y visor de PDF diseñado bajo una filosofía minimalista y de alto rendimiento. En lugar de saturar la interfaz con herramientas de edición complejas y flujos pesados, Adobo se enfoca en proporcionar una lectura suave, instantánea y agradable. 

> [!NOTE]
> **¿Por qué "Adobo"?** Además de su enfoque técnico, el nombre es un homenaje al **Adobo Arequipeño**, un plato emblemático y tradicional de la gastronomía peruana originario de la región de Arequipa. Este potaje de cerdo marinado en concho de chicha de jora y especias se consume tradicionalmente muy temprano por las mañanas, dando origen a la célebre costumbre y expresión local de los **"Lunes de Adobo.."**.

Este documento explica en detalle la arquitectura del visor, el pipeline de procesamiento de documentos y las soluciones de ingeniería de software implementadas para erradicar cuellos de botella clásicos como la latencia de scroll, el parpadeo en el zoom y la sobrecarga de la CPU.

---

## 🏗️ Arquitectura del Pipeline de Datos

Adobo Reader está estructurado como un **pipeline de transformación de datos unidireccional** que convierte bytes crudos de archivos PDF en gráficos renderizados en pantalla mediante la tarjeta de video (GPU):

```
  [ Archivo .pdf ] 
         │
         ▼
  ┌────────────────────────┐
  │ 1. Parser & Lexer      │ <--- Tokeniza y arma el árbol de objetos usando la tabla XREF
  └────────────────────────┘
         │ (Estructura de objetos indexada con Lazy Loading)
         ▼
  ┌────────────────────────┐
  │ 2. Decompressor/Filter │ <--- Descomprime streams de datos e imágenes (FlateDecode)
  └────────────────────────┘
         │ (Content Streams en texto claro / Implosión de filtros de imágenes)
         ▼
  ┌────────────────────────┐
  │ 3. Interpreter         │ <--- Máquina de estados gráficos (BT, ET, Td, Tj, matrices CTM)
  └────────────────────────┘
         │ (Lista de DrawCommands vectoriales listos para GPU)
         ▼
  ┌────────────────────────┐
  │ 4. Rasterizer (Vello)  │ <--- Envía fórmulas matemáticas y texturas directamente a la GPU
  └────────────────────────┘
```

### Detalle de Componentes
1. **Parser & Lexer (Acceso a Datos):** El estándar PDF no se lee de arriba a abajo, sino de atrás hacia adelante. El parser localiza el `trailer` y la tabla **XREF (Cross-Reference Table)**. Esto permite el acceso directo a cualquier objeto del archivo por su ID mediante *Lazy Loading*, evitando cargar todo el documento en la memoria RAM.
2. **Decodificador de Filtros (Capa de Transformación):** La información textual y las imágenes están comprimidas en objetos `Stream`. El decodificador identifica filtros como `FlateDecode` (basado en zlib/deflate) y los procesa para obtener las instrucciones nativas del documento en texto claro.
3. **Intérprete de Contenido (Máquina de Estados):** Procesa el *Content Stream* (un lenguaje tipo PostScript). El intérprete gestiona una máquina de estados gráficos (coordenadas, rotaciones, escala actual, colores y fuentes tipográficas) y los traduce a un conjunto estructurado de primitivas de dibujo (`DrawCommand::Text`, `DrawCommand::Image`, `DrawCommand::Path`).
4. **Rasterizador y GPU (Presentación):** En lugar de rasterizar en la CPU, los comandos vectoriales se transforman a escenas de **Vello** que se dibujan directamente en la GPU usando **wgpu** a velocidades de renderizado de nivel de hardware (de 100ms a **2ms**).

---

## ⚡ Problemas de Rendimiento y Soluciones Implementadas

### 1. Scroll Fluido a 60+ FPS (Separación de Hilos y Caching)
> [!IMPORTANT]
> **El problema (Antipatrón):** Si el hilo principal de la interfaz de usuario (UI Thread) se encarga de descomprimir flujos de datos del PDF, interpretar comandos PostScript y rasterizar píxeles en caliente durante el scroll, la aplicación experimenta caídas drásticas de frames y latencia perceptible.

#### La Solución en Adobo:
* **Separación de Concernimientos:** El hilo de la UI (`winit`) solo procesa la entrada del ratón y redibuja elementos gráficos ya procesados.
* **Caché de Imágenes y Texturas:** Las páginas se rasterizan una sola vez y se almacenan en RAM como texturas/imágenes. Durante el scroll continuo, el motor gráfico solo ajusta las coordenadas verticales de dibujo de estas imágenes en la escena, una operación instantánea para la GPU.
* **Hilos de Renderizado en Segundo Plano (Workers):** La descompresión e interpretación del PDF ocurren en hilos secundarios. Si una página aún no está lista, la UI dibuja un indicador de carga sin bloquear el scroll.

---

### 2. Gestión Eficiente de RAM (Caché LRU)
> [!WARNING]
> Guardar los buffers de píxeles o texturas de un PDF de cientos de páginas saturaría rápidamente la memoria del sistema.

#### La Solución en Adobo:
Se implementó una **Caché LRU (Least Recently Used)** con una capacidad dinámica (típicamente de 4 a 6 páginas).
* **Lazy Cargar + Caché Temporal:** El sistema busca la página requerida en el contenedor de caché. Si es un *Hit*, se entrega instantáneamente (0 ms). Si es un *Miss*, se procesa asíncronamente y se inserta en el caché.
* **Desalojo Automático:** Cuando el caché alcanza su capacidad máxima, el componente elimina de forma segura la página que lleva más tiempo sin visualizarse (`drop`), garantizando un consumo de RAM constante e independiente de la longitud del documento.

---

### 3. Comportamiento Humano: Sesgo de Lectura y Margen Asimétrico
> [!NOTE]
> Los seres humanos leemos de arriba a abajo. Esto provoca que el 90% del tiempo el scroll se mueva hacia abajo y solo un 10% hacia arriba.

#### La Solución en Adobo:
* **Margen Asimétrico Dinámico:** En lugar de mantener una precarga simétrica estática (como 2 páginas arriba y 2 abajo), el lector ajusta su buffer según la dirección del scroll:
  * **Hacia abajo:** Precarga **3 páginas hacia adelante y 1 hacia atrás**. Esto incrementa la tolerancia frente a scrolls rápidos hacia abajo, dejando el búfer mínimo indispensable arriba por si el usuario decide releer el párrafo inmediato anterior.
  * **Hacia arriba:** Invierte la relación a **3 páginas hacia atrás y 1 hacia adelante**.
* **Estados de Página:**
  1. **Visible:** Renderizadas en pantalla.
  2. **Pre-render (Caché Activo):** Rasterizadas en segundo plano esperando interacciones inmediatas.
  3. **Virtualizadas:** Solo existen como metadatos (ancho, alto e índices) para dimensionar las barras de scroll y saltos, consumiendo **0 bytes** de RAM gráfica.

---

### 4. Zoom Continuo sin Parpadeo (Pinch-to-Zoom)
> [!TIP]
> Al alterar el nivel de zoom, recalcular vectores a una nueva resolución toma unos milisegundos. Borrar la pantalla para esperar este renderizado provoca un molesto parpadeo.

#### La Solución en Adobo:
* **Blitting Temporal:** Cuando el usuario hace zoom, en lugar de borrar la pantalla o pedir un renderizado inmediato del PDF, el motor gráfico **estira o encoge la imagen ya existente** del caché aplicando escalado por hardware con la GPU.
* **Muestreo Lineal (Bilineal):** La textura estirada se ve ligeramente borrosa por unos milisegundos, pero la lectura es continua y nunca desaparece el documento.
* **Zoom Debounce:** El motor de renderizado del PDF no recibe peticiones intermedias de escala. Espera a que el usuario se detenga durante **200 ms** para disparar **una única petición** del zoom final al hilo de fondo. Una vez que el buffer de alta definición está listo, se realiza una transición suave (*fade-in* de 100 ms) sustituyendo la textura estirada por la nítida.

---

### 5. Priorización y Cancelación Activa de Tareas
> [!IMPORTANT]
> Un canal estándar `mpsc` funciona como cola FIFO. Si hay 4 peticiones de precarga de scroll pendientes, un clic inmediato de navegación del usuario tardaría en responder hasta vaciar la cola actual.

#### La Solución en Adobo:
* **Cola de Prioridades:** Los comandos se dividen en **Baja prioridad** (precarga/scroll) y **Alta prioridad** (saltos de página directos y clics en botones de navegación).
* **Cancelación Activa por Flag Atómico:** Cuando se detecta un evento de alta prioridad, se dispara un flag de cancelación (`Arc<AtomicBool>`). Los hilos secundarios consultan este flag en su ciclo interno de dibujo: si se marca como cancelado, abortan instantáneamente la tarea actual de baja prioridad a mitad de proceso, liberan la CPU y atienden de inmediato la petición urgente de la interfaz.
* **Hover Prefetching:** Utilizando los eventos `CursorMoved` de `winit`, si el puntero se posiciona sobre los botones de navegación, se inicia en segundo plano el pre-renderizado de la página destino de forma anticipada antes de que ocurra el clic físico del ratón.

---

### 6. Optimizaciones Críticas del Sistema (GPU, Rayon y memmap2)
* **Rasterizado Acelerado por GPU (Vello):** Reemplazar `tiny-skia` (render por software en CPU) con **Vello** y **Wgpu** (GPU de bajo nivel) permite delegar operaciones de rasterizado complejas (fuentes tipográficas, trazado de curvas Bezier e imágenes) a miles de núcleos de procesamiento en paralelo. El renderizado medio baja de 100ms a **2ms**.
* **Procesamiento en Paralelo con Rayon:** Un hilo *Worker* único desperdicia procesadores multinúcleo. Con **Rayon**, las páginas del margen de pre-renderizado se procesan concurrentemente utilizando hilos de trabajo paralelos.
* **Acceso I/O con Memory Mapping (memmap2):** La lectura secuencial de archivos con `std::fs::File` introduce latencias I/O debido al sistema de archivos del sistema operativo. Adobo utiliza `memmap2` para mapear los bytes del PDF directamente a la memoria virtual de la CPU. El acceso a las tablas XREF y streams de datos ocurre instantáneamente a nivel de hardware.

---

## 📂 Estructura de Código

* **[src/main.rs](file:///C:/Users/jreyn/OneDrive/Documents/Projects/ufreader/src/main.rs):** Punto de entrada. Procesa los argumentos del CLI e inicializa el bucle gráfico.
* **[src/gui_vello.rs](file:///C:/Users/jreyn/OneDrive/Documents/Projects/ufreader/src/gui_vello.rs):** Lógica del visor acelerado por hardware mediante la API de Vello y Wgpu.
* **[src/parser.rs](file:///C:/Users/jreyn/OneDrive/Documents/Projects/ufreader/src/parser.rs):** Analizador sintáctico del PDF, decodificador de filtros y lector de tablas XREF.
* **[src/interpreter.rs](file:///C:/Users/jreyn/OneDrive/Documents/Projects/ufreader/src/interpreter.rs):** Evaluador del árbol de comandos de visualización y máquina de estados gráficos del PDF.
* **[src/object.rs](file:///C:/Users/jreyn/OneDrive/Documents/Projects/ufreader/src/object.rs):** Modelado y tipado de objetos del formato PDF (diccionarios, números, nombres, streams).
* **[src/db.rs](file:///C:/Users/jreyn/OneDrive/Documents/Projects/ufreader/src/db.rs):** Persistencia de estado y configuración (lecturas recientes, dimensiones).

---

## 🔮 Próximo Lanzamiento (Roadmap)
Para las siguientes versiones de Adobo Reader, se planea expandir el visor con funcionalidades colaborativas y de estudio, manteniendo la misma filosofía de alto rendimiento:
1. **Capacidades de Apunte (Anotaciones):** Implementación de herramientas para subrayar, resaltar y añadir notas de texto o dibujos a mano alzada directamente sobre los vectores del PDF.
2. **Salas de Lectura:** Espacios compartidos y sincronizados en tiempo real para que múltiples usuarios puedan leer el mismo documento, compartir apuntes y debatir en salas virtuales.

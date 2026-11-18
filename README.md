# Medidor de recursos

Monitor ligero para Windows, escrito en Rust, para averiguar **por qué un PC pierde FPS**: ¿se calienta?, ¿hay un proceso en segundo plano comiéndose la CPU o la GPU?, ¿se llena la RAM o la VRAM?, ¿es la red?

Se inicia desde una ventana (o desde la consola), se queda en la bandeja del sistema mientras juegas y al terminar genera un **informe HTML con gráficas y un diagnóstico**, además de un CSV con todas las muestras.

![Interfaz](distribuir/captura-gui.png)

- Dos ejecutables de ~1,2 MB, sin instalación: `medidor-gui.exe` (ventana + icono en la bandeja) y `medidor-recursos.exe` (consola).
- El monitor gasta **~0,5 % de CPU y ~18 MB de RAM**, con prioridad baja. La ventana se puede cerrar sin detener la medición.
- El informe es un único HTML que funciona sin internet.

[![CI](https://github.com/borjajoart/medidor-recursos/actions/workflows/ci.yml/badge.svg)](https://github.com/borjajoart/medidor-recursos/actions/workflows/ci.yml)

## Qué mide

Cada 2 segundos (configurable):

| Dato | Fuente |
|---|---|
| CPU, RAM, disco y red (subida/bajada) | Windows (`sysinfo`) |
| Procesos: CPU, RAM y disco, agrupados por ejecutable | Windows |
| **Uso de GPU por proceso** | Contadores de rendimiento `GPU Engine` |
| **FPS, 1 % low, frametime máximo y tirones** del juego | ETW (los mismos eventos que PresentMon). Requiere administrador |
| Ping y paquetes perdidos | ICMP a 1.1.1.1 (configurable) |
| Ventana activa (normalmente el juego) | Win32 |
| Temperatura, frecuencia y consumo de CPU | LibreHardwareMonitor (WMI) |
| Temperatura, *hot spot*, uso, frecuencia, consumo y VRAM de GPU | LibreHardwareMonitor o driver NVIDIA (NVML) |
| Ventiladores (RPM), discos y placa base | LibreHardwareMonitor |

El informe trae:

- **Resumen** en dos líneas y los datos clave de la sesión (FPS medios, temperaturas máximas, ping…).
- **Hallazgos** numerados por gravedad: calor y *throttling*, caídas de FPS y lo que coincidía con cada una, CPU o GPU saturadas y por quién, RAM/VRAM llenas, procesos en segundo plano, picos, disco, red, ventiladores parados.
- **Caídas de FPS** una por una, con la hora, cuánto bajó y las causas visibles en ese momento (otro proceso usando la GPU, un pico de CPU, throttling, VRAM llena…).
- **Marcas**: los momentos que marcaste con **Ctrl+Alt+M** mientras jugabas («aquí ha dado un tirón»), con lo que pasaba en ese instante.
- **Gráficas** de FPS, temperaturas, uso, frecuencias, consumo, ventiladores, ping, tráfico y disco, con el cursor sincronizado entre todas y un detalle al pasar el ratón (ventana activa, procesos que más CPU y GPU usaban, marcas).
- **Tablas** de procesos, picos, ventanas activas y memoria.

Hay un informe de ejemplo con datos simulados en [`ejemplo/informe_ejemplo.html`](ejemplo/informe_ejemplo.html) (descárgalo y ábrelo con el navegador; GitHub no muestra HTML).

## Funciones

- **Icono en la bandeja** con el color de la temperatura (gris esperando, verde, naranja cerca del límite, rojo por encima). Tooltip con temperaturas y FPS; menú para marcar un momento, abrir el panel o detener.
- **Avisos** en Windows cuando algo va mal durante la partida (CPU/GPU caliente, RAM o VRAM llenas, un proceso que se dispara en segundo plano). Como mucho uno de cada tipo cada 10 minutos.
- **Marcas** con **Ctrl+Alt+M** (funciona dentro del juego), con el botón del panel, el menú del icono o `medidor-recursos marcar`.
- **Esperar al juego**: le das la lista de ejecutables y no mide nada hasta que abres uno; para al cerrarlo. Un informe por partida.
- **Iniciar con Windows**: se queda esperando a los juegos de la lista desde que inicias sesión.
- **Comparar sesiones**: elige dos CSV y genera una página con la diferencia (antes/después de limpiar el PC, de cambiar la pasta térmica, de cerrar Discord…).
- Las opciones del panel se guardan en `%APPDATA%\MedidorRecursos\config.json`.

## Instalación

### Opción A: descargar

1. En [Releases](../../releases) descarga `medidor-recursos-windows.zip`.
2. Descomprímelo en cualquier carpeta.
3. Abre **`medidor-gui.exe`**. La primera vez SmartScreen puede avisar porque el exe no está firmado: *Más información → Ejecutar de todas formas*.

### Opción B: compilar

Requisitos: Windows 10/11 y [Rust](https://rustup.rs) (toolchain MSVC, la predeterminada).

```bash
git clone https://github.com/borjajoart/medidor-recursos.git
```

```bash
cd medidor-recursos
```

```bash
cargo build --release
```

Los dos ejecutables quedan en `target\release\`. `build.rs` incrusta el manifiesto de Windows de la interfaz (`res/medidor-gui.manifest`, sin él no arranca) y los iconos (`res/recursos.rc`; se regeneran con `res/iconos.ps1`).

Cada etiqueta `v*` que se sube a GitHub compila, pasa los tests y publica el zip en Releases automáticamente (`.github/workflows/release.yml`).

### Temperaturas: LibreHardwareMonitor (recomendado)

Windows no da las temperaturas reales a programas normales. Para tenerlas:

1. Descarga [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases) y descomprímelo.
2. Ábrelo con **clic derecho → Ejecutar como administrador** y déjalo abierto (puede estar minimizado).
3. Inicia el medidor. Si abres LibreHardwareMonitor después, se detecta en menos de 30 s.

Sin él todo lo demás funciona; la GPU solo se lee si es NVIDIA.

### FPS: permiso de administrador

Para leer los FPS, Windows exige que el monitor se ejecute como administrador. En el panel, marca **«Medir FPS»**: al pulsar Iniciar, Windows mostrará el aviso de Control de cuentas. Sin ese permiso se mide todo lo demás y el informe indica que no hay FPS.

## Probarlo

**Con la interfaz:**

1. Abre `medidor-gui.exe`, deja las opciones por defecto y pulsa **Iniciar**.
2. En **Estado** aparece «Midiendo» con el uso de CPU/GPU/RAM, el proceso que más consume y la ventana activa. Junto al reloj de Windows sale el icono verde.
3. Abre algo pesado (un juego, un benchmark, un vídeo 4K). Pulsa **Ctrl+Alt+M** un par de veces para dejar marcas.
4. Pulsa **Detener e informe** (o *Detener y generar informe* en el menú del icono). El informe se abre en el navegador y queda en la carpeta `informes`.

**Con la consola (1 minuto):**

```bash
medidor-recursos.exe --duracion 1m --intervalo 1
```

En segundo plano y esperando a un juego:

```bash
medidor-recursos.exe -b --esperar cs2.exe
```

```bash
medidor-recursos.exe estado
```

```bash
medidor-recursos.exe detener
```

Comparar dos sesiones:

```bash
medidor-recursos.exe comparar informes\datos_A.csv informes\datos_B.csv
```

La interfaz y la consola comparten el mismo monitor: se puede iniciar en una y detener en la otra.

**Tests:**

```bash
cargo test
```

Comprueban el análisis (caídas de FPS y su causa, throttling, tramos, app principal), el cálculo de FPS, la comparación de CSV y los argumentos, y generan `ejemplo/informe_ejemplo.html` con una sesión simulada.

## Uso por consola

```text
medidor-recursos [opciones]              Monitoriza hasta Ctrl+C (o hasta --duracion)
medidor-recursos detener                 Detiene el monitor en segundo plano
medidor-recursos estado                  Indica si hay un monitor en marcha
medidor-recursos marcar                  Añade una marca (también Ctrl+Alt+M)
medidor-recursos comparar a.csv b.csv    Compara dos sesiones [-o salida.html]

-d, --duracion <t>     90s, 30m, 2h, 1h30m (número solo = minutos)
-i, --intervalo <s>    Segundos entre muestras (por defecto 2)
-o, --salida <dir>     Carpeta de informes (por defecto 'informes' junto al .exe)
    --umbral-cpu <°C>  Temperatura de CPU considerada alta (85)
    --umbral-gpu <°C>  Temperatura de GPU considerada alta (83)
    --ping <host>      Host para el ping (1.1.1.1; 'no' para desactivar)
    --sin-fps          No intenta medir FPS
    --esperar <exes>   Espera a que se abra uno de estos juegos (separados por comas)
-b, --segundo-plano    Sin ventana; se para con 'detener'
    --no-abrir         No abrir el informe al terminar
```

El manual para usuarios no técnicos (cómo leer el informe y qué hacer según el resultado) está en [`distribuir/MANUAL.md`](distribuir/MANUAL.md).

## Limitaciones

- **Solo Windows** (WMI, ETW, contadores de rendimiento y Win32).
- **FPS:**
  - Necesitan administrador (sesión ETW en tiempo real).
  - Solo DirectX 9/10/11/12 (eventos *Present* de DXGI y D3D9). **Vulkan y OpenGL no se cuentan**; con esos juegos el informe sale sin FPS.
  - Se cuentan las llamadas a *Present* del proceso, igual que PresentMon en su modo básico: con generación de fotogramas (DLSS 3/FSR 3) o VSync forzado por el driver la cifra puede no coincidir con la del overlay.
  - No se ha podido probar en el equipo de desarrollo (sin juegos DirectX a mano con permisos de administrador); el cálculo está cubierto por tests, la captura ETW no.
- **Temperaturas dependientes de LibreHardwareMonitor.** Sin él: GPU NVIDIA por NVML, GPU AMD/Intel sin datos, CPU solo con la zona ACPI (requiere administrador y en sobremesa suele ser un valor fijo sin sentido; el informe lo avisa). Leer los sensores directamente exigiría instalar un driver de kernel, que es justo lo que hace LibreHardwareMonitor.
- **Sensores sin validar en hardware variado.** Se ha desarrollado en un i5-6500 + Radeon RX 560 sin LibreHardwareMonitor. Los nombres de los sensores cambian entre placas y versiones, así que en algún equipo puede faltar algún valor (consumo, frecuencia, ventiladores). Si pasa, abre un issue con una captura de LibreHardwareMonitor.
- **Ventiladores:** un ventilador a 0 rpm solo se señala si coincide con temperatura alta (muchos se paran solos en reposo). Algunos conectores de la placa aparecen aunque no tengan nada conectado.
- **GPU por proceso:** porcentaje del motor más ocupado de cada proceso (3D, copia, vídeo…), no una suma; los valores pueden superar ligeramente el uso global. Si los contadores `GPU Engine` no están disponibles (drivers antiguos), no hay datos por proceso.
- **Una sola GPU.** Con varias se elige la dedicada (NVIDIA > AMD > Intel); con NVML, la GPU 0.
- **Resolución temporal.** Con 2 s de intervalo, picos de menos de 2 s se promedian. Los FPS sí cuentan todos los fotogramas del intervalo.
- **Atribución aproximada.** Los responsables salen del top de procesos de cada muestra; los del mismo nombre se suman (todos los `svchost.exe`, todas las pestañas de `chrome.exe`).
- **Throttling de CPU por heurística**: caída de frecuencia > 20 % respecto al P95 con carga ≥ 50 % y temperatura cerca del umbral. Puede no detectar límites de potencia (PL1/PL2). En NVIDIA se usa además el motivo que informa el driver.
- **«Segundo plano» = «no es la ventana activa».** Con dos monitores o alt-tab la clasificación puede no coincidir con lo que consideras el juego.
- **Avisos:** Windows los oculta durante juegos a pantalla completa (Asistente de concentración); quedan en el Centro de actividades.
- **Esperar al juego:** una partida de menos de ~20 s que se cierra sola (un launcher) no genera informe. Hay que escribir el nombre exacto del ejecutable (se ve en el Administrador de tareas → Detalles).
- **Inicio con Windows con FPS:** si «Medir FPS» está activado, el aviso de administrador sale en cada inicio de sesión.
- **Administrador con otra cuenta:** si el usuario no es administrador y escribe la contraseña de otra cuenta en el aviso, el monitor usa la carpeta temporal de esa cuenta y el panel no lo verá. Se puede detener desde el icono de la bandeja.
- **Guardado:** el CSV se escribe en cada muestra; el HTML cada 5 minutos y al terminar. Si se mata el proceso puede faltar el final en el informe (en el CSV no).
- **Un monitor a la vez** (archivos de control en `%TEMP%`).
- **Ejecutable sin firma.** Firmarlo requiere un certificado de pago; SmartScreen y algunos antivirus pueden avisar la primera vez.
- **Interfaz básica:** ventana Win32 de tamaño fijo. El botón Manual abre `MANUAL.md` en el Bloc de notas.

## Estructura

```text
src/
  lib.rs             Librería compartida por los dos ejecutables
  cli.rs             Opciones y argumentos
  monitor.rs         Bucle de muestreo, procesos, CSV, marcas, avisos, control (detener/estado)
  sensors.rs         LibreHardwareMonitor/OpenHardwareMonitor (WMI), NVIDIA NVML, ACPI
  fps.rs             FPS por ETW (DXGI/D3D9)
  gpu_proc.rs        Uso de GPU por proceso (PDH)
  net.rs             Ping ICMP
  analysis.rs        Caídas de FPS, episodios, throttling, causas y hallazgos (funciones puras, con tests)
  report.rs          Genera el informe HTML
  compare.rs         Comparación de dos sesiones
  informe.html, comparar.html, estilo.css, charts.js   Plantillas (sin dependencias externas)
  main.rs            medidor-recursos.exe
  bin/medidor-gui.rs medidor-gui.exe: panel, icono de bandeja e inicio con Windows
res/                 Manifiesto e iconos
.github/workflows/   CI (tests en cada push) y publicación de releases
distribuir/          Manual y captura para usuarios
ejemplo/             Informe de ejemplo generado por `cargo test`
```

## Licencia

[MIT](LICENSE).

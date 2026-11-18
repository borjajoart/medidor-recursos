# Medidor de recursos — Manual de uso

Programa pequeño (menos de 1 MB, sin instalación) que vigila el PC mientras juegas y al terminar genera un **informe con gráficas** que te dice:

- Qué temperatura alcanzaron la **CPU**, la **GPU** y los **discos**, y a qué hora.
- **Qué procesos** consumían más en cada momento, y quién estaba gastando CPU cuando subió la temperatura.
- Si hay **programas en segundo plano** (antivirus, Windows Update, navegador, launchers…) que se comen la CPU mientras juegas, y los **picos** que pueden causar tirones.
- Si la CPU o la GPU **bajaron su frecuencia por calor** (*thermal throttling*), que es una causa típica de bajadas de FPS.
- Si la **RAM** se llenó o el **disco** estuvo muy ocupado.

Consume muy poco: en las pruebas, alrededor del **0,5 % de CPU y 15 MB de RAM**. Además se ejecuta con prioridad baja para no quitarle recursos al juego.

---

## 1. Antes de empezar: temperaturas (recomendado)

Windows no ofrece las temperaturas reales a los programas normales. Para tenerlas, el monitor lee los sensores a través de **LibreHardwareMonitor**, un programa gratuito y de código abierto.

1. Descárgalo de su página oficial en GitHub: busca «LibreHardwareMonitor releases» y baja el `.zip` de la última versión.
2. Descomprímelo en una carpeta (por ejemplo `C:\LibreHardwareMonitor`).
3. Abre `LibreHardwareMonitor.exe` con **clic derecho → Ejecutar como administrador**.
4. **Déjalo abierto** (puedes minimizarlo) mientras usas el medidor.

Opcional, para no tener que acordarte: en LibreHardwareMonitor activa *Options → Start Minimized*, *Minimize To Tray* y *Run On Windows Startup*.

> **Sin LibreHardwareMonitor** el medidor funciona igual para procesos, CPU, RAM y disco.
> - Con gráfica **NVIDIA**, la temperatura y el uso de la GPU se leen directamente del driver.
> - Con **AMD o Intel**, sin LibreHardwareMonitor no habrá datos de la GPU.
> - La temperatura de CPU solo saldrá si Windows la ofrece (zona térmica ACPI, a menudo un valor fijo y poco fiable, y requiere ejecutar como administrador).

---

## 2. Uso rápido (doble clic)

En la carpeta hay tres archivos `.bat`:

| Archivo | Qué hace |
|---|---|
| **1 - Monitorizar (con ventana).bat** | Abre una ventana que muestra los datos en directo. Juega normalmente y, al terminar, pulsa **Ctrl+C** en esa ventana. Se genera el informe y se abre en el navegador. (Después Windows puede preguntar «¿Desea terminar el trabajo por lotes?»: responde **S**.) |
| **2 - Monitorizar en segundo plano.bat** | Arranca el monitor **sin ventana**. Puedes cerrar todo y jugar. |
| **3 - Detener y ver informe.bat** | Detiene el monitor en segundo plano, genera el informe y lo abre. |

Flujo típico para cazar las bajadas de FPS:

1. Abre LibreHardwareMonitor (como administrador).
2. Doble clic en **2 - Monitorizar en segundo plano.bat**.
3. Juega como siempre, al menos 30–60 minutos. **Apunta la hora** si notas una bajada de FPS fuerte.
4. Al terminar, doble clic en **3 - Detener y ver informe.bat**.
5. En el informe, pasa el ratón por las gráficas a la hora que apuntaste.

Los informes se guardan en la carpeta **`informes`**, junto al programa.

> Si cierras la ventana con la X en vez de usar Ctrl+C, no pasa nada grave: el informe se guarda automáticamente cada 5 minutos y el CSV se escribe en cada muestra.

---

## 3. Cómo leer el informe

**Arriba (resumen):** temperatura máxima y media de CPU y GPU, uso medio de CPU y GPU, RAM máxima y la **app principal** (la ventana que más tiempo estuvo activa, normalmente el juego).

**Diagnóstico:** conclusiones automáticas ordenadas por gravedad:

- 🔴 **Grave**: temperatura crítica, throttling, RAM llena…
- 🟠 **Aviso**: temperaturas altas, procesos en segundo plano que consumen, CPU al 100 %…
- 🔵 **Info**: datos útiles (p. ej. «la GPU va al máximo», que en juegos es normal).
- 🟢 **OK**: todo correcto.

Muchos procesos conocidos traen una explicación y qué hacer (por ejemplo `MsMpEng.exe` → análisis del antivirus Defender).

**Gráficas** (temperaturas, uso, frecuencias, consumo eléctrico y disco):

- **Pasa el ratón** por encima: verás la hora exacta, los valores, **la ventana activa y los 5 procesos que más CPU usaban en ese instante**. Las líneas de todas las gráficas se mueven a la vez.
- Las **zonas sombreadas** marcan episodios: rojo = CPU caliente, naranja = GPU caliente, morado = CPU saturada (más del 90 %).
- Las líneas discontinuas son los límites de temperatura configurados.
- Si en la gráfica de **frecuencias** la línea de la CPU o la GPU baja justo cuando la temperatura está arriba, es *throttling*: el PC se está frenando por calor.

**Episodios detectados:** cada vez que hubo calor, CPU saturada o RAM llena: cuándo empezó, cuánto duró, valor máximo, qué ventana estaba activa y **qué procesos usaron más CPU** desde 30 s antes del inicio.

**Procesos que más CPU consumieron:** media, máximo, consumo **en segundo plano** (cuando no eran la ventana activa), RAM, disco y % de tiempo en primer plano. `×14` indica que había 14 procesos con ese nombre (típico de navegadores).

**Picos de CPU en segundo plano:** procesos que, sin ser la ventana activa, superaron el 15 % de la CPU total. Son los principales sospechosos de tirones.

---

## 4. Qué hacer según el resultado

| Lo que ves | Causa probable | Solución |
|---|---|---|
| CPU > 85–90 °C, o «thermal throttling» | Refrigeración insuficiente | Limpiar polvo del disipador y ventiladores, cambiar la pasta térmica si tiene años, mejorar el flujo de aire, revisar la curva de ventiladores en la BIOS. |
| GPU > 83–85 °C, o «GPU limitada» | Gráfica sucia o caja mal ventilada | Limpiar ventiladores de la gráfica, curva de ventilador más agresiva (MSI Afterburner / AMD Adrenalin), limitar FPS. |
| Picos de `MsMpEng.exe` | Antivirus analizando | Programar el análisis a otra hora o añadir la carpeta del juego a exclusiones. |
| Picos de `TiWorker.exe`, `TrustedInstaller.exe`, `MoUsoCoreWorker.exe` | Windows Update | Dejar que termine de actualizar o pausar las actualizaciones al jugar. |
| Navegador, Discord, launchers con CPU alta | Programas abiertos | Cerrarlos o desactivar la aceleración por hardware / overlays. |
| RAM > 90 % | Poca memoria | Cerrar programas, ampliar RAM. |
| CPU al 100 % por el propio juego | Cuello de botella de CPU | Bajar opciones que cargan la CPU (distancia de dibujado, población, físicas) o limitar FPS. |
| GPU al ~97–100 % siempre | Normal en juegos: los FPS dependen de la gráfica | Bajar calidad gráfica/resolución si quieres más FPS. |
| Proceso desconocido con CPU alta y constante | Podría ser malware (p. ej. un minero) | Buscar el nombre del proceso en internet y pasar un antivirus. |

---

## 5. Uso desde la consola (opciones avanzadas)

Abre una terminal en la carpeta del programa:

```bash
medidor-recursos.exe --duracion 1h
```

| Opción | Descripción | Por defecto |
|---|---|---|
| `-d`, `--duracion <t>` | Tiempo de monitorización: `90s`, `30m`, `2h`, `1h30m` (un número solo = minutos). Sin ella, hasta Ctrl+C o `detener`. | sin límite |
| `-i`, `--intervalo <s>` | Segundos entre muestras (0.5 – 600). | 2 |
| `-o`, `--salida <carpeta>` | Dónde guardar los informes. | `informes` junto al .exe |
| `--umbral-cpu <°C>` | Temperatura de CPU considerada alta. | 85 |
| `--umbral-gpu <°C>` | Temperatura de GPU considerada alta. | 83 |
| `-b`, `--segundo-plano` | Ejecutar sin ventana. | — |
| `--no-abrir` | No abrir el informe al terminar. | — |
| `detener` | Detiene el monitor en segundo plano y espera a que guarde el informe. | — |
| `estado` | Dice si hay un monitor en marcha y dónde guardará el informe. | — |
| `-h`, `--ayuda` | Muestra la ayuda. | — |

Ejemplos:

```bash
medidor-recursos.exe --segundo-plano --duracion 2h
```

```bash
medidor-recursos.exe --intervalo 1 --umbral-cpu 80
```

```bash
medidor-recursos.exe estado
```

---

## 6. Archivos generados

En la carpeta `informes`:

- `informe_AAAA-MM-DD_HH-MM-SS.html` — el informe. Se abre con cualquier navegador, **funciona sin internet** y se puede enviar a otra persona (es un único archivo).
- `datos_AAAA-MM-DD_HH-MM-SS.csv` — todas las muestras (hora, uso, temperaturas, frecuencias, ventana activa y top 3 de procesos). Separado por `;` y con coma decimal, para abrirlo directamente con Excel en español.

---

## 7. Preguntas frecuentes

**Windows dice «Windows protegió su PC» al abrirlo.**
El programa no está firmado digitalmente. Pulsa *Más información → Ejecutar de todas formas*.

**No salen temperaturas.**
Comprueba que LibreHardwareMonitor está abierto **como administrador** antes de iniciar el medidor (si lo abres después, el medidor lo detecta en menos de 30 s). Mira la línea «Sensores» al principio del informe para ver qué fuentes se usaron.

**¿Puede medir los FPS?**
No directamente. Por eso es útil apuntar la hora de las bajadas y buscarla en las gráficas. Si quieres además FPS, usa el overlay de Steam, NVIDIA o AMD a la vez; no interfiere.

**¿Cuánto tiempo puedo dejarlo?**
Horas sin problema. En sesiones muy largas las gráficas se resumen (conservando los picos) para que el informe no pese demasiado; el análisis y el CSV usan todos los datos.

**«Ya hay un monitor en marcha».**
Solo puede haber uno a la vez. Ejecuta **3 - Detener y ver informe.bat** o `medidor-recursos.exe detener`.

---

## 8. Compilar desde el código (opcional)

Requiere Rust (https://rustup.rs). En la carpeta del proyecto:

```bash
cargo build --release
```

El ejecutable queda en `target\release\medidor-recursos.exe`. `cargo test` genera además un informe de ejemplo con datos simulados en `ejemplo\informe_ejemplo.html`.

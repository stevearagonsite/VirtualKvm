# Diseño: no perder nunca el control de una máquina

> Estado: **propuesto** (2026-11-14). Las secciones "Hoy" y "Qué hacer si se traba" describen lo que ya funciona.

## Problema

Cuando una máquina **captura** (manda tu mouse y teclado a la otra), se traga el input local. Si la otra
máquina se cuelga, se reinicia (por ejemplo, durante una actualización) o la red se corta, la que
captura puede tardar en darse cuenta. Mientras tanto el mouse y el teclado no responden. Tiene que ser
imposible quedar sin control de una máquina, sobre todo de la Windows.

## Hoy

| Mecanismo | Dónde | Efecto |
|---|---|---|
| Combinación de emergencia: **Ctrl + Shift + Win + Alt** (Mac: **Ctrl + Shift + Cmd + Option**) | La máquina que captura | Suelta la captura y devuelve el control a esa máquina |
| Error de envío o conexión cerrada (sin respuesta a pings durante ~2 s) | La máquina que captura | Suelta la captura |
| Liberar teclas al desconectar | La máquina que recibe | Suelta las teclas que quedaron apretadas |
| Hooks de bajo nivel con timeout del sistema | Windows | Si el proceso se cuelga, Windows saltea el hook y el input vuelve (con algo de demora) |
| Tap de eventos con timeout del sistema | macOS | Si el proceso se cuelga, macOS desactiva el tap (`TapDisabledByTimeout`) y el código libera el estado |

## Propuesta (capas, de más útil a último recurso)

1. **El input físico manda.** Si una máquina está siendo controlada desde la otra y se mueve su mouse
   o se toca su teclado, recupera el control al instante (manda `Leave` al peer). Es el comportamiento
   de Mouse Without Borders.
2. **Recuperación automática.** Si la máquina que captura no recibe ninguna respuesta del peer (ack,
   pong o evento) durante **1,5 s**, suelta la captura sola. No depende de que aparezca un error de red.
3. **Teclas liberadas por inactividad.** Si la máquina que recibe tiene teclas apretadas y no le llega
   nada del peer durante **2 s**, las suelta todas (y también los modificadores). Así no queda un Ctrl pegado.
4. **Rescate desde la otra máquina.** Opción en el ícono: "Recuperar el control de \<peer\>". Manda un
   evento `Panic` y el peer suelta la captura, libera las teclas y **pausa VirtualKvm 60 s**.
5. **Combinación de emergencia** (ya existe, punto "Hoy").
6. **Último recurso en Windows: Ctrl + Alt + Supr.** Es la secuencia de atención segura: ningún programa
   la puede interceptar. Desde ahí: Administrador de tareas → finalizar `vkvm.exe`. Al terminar el
   proceso, Windows quita sus hooks y el mouse y el teclado vuelven al instante.

Recomendación: implementar 1, 2, 3 y 4. Entre las cuatro cubren casi todos los casos sin que el usuario
tenga que recordar nada.

## Regla para actualizar (desarrollo)

Antes de instalar una versión nueva: **apagar VirtualKvm en las dos máquinas** desde el ícono, instalar y
volver a prender. Los instaladores (`scripts/dev/*-install-autostart.*`) detienen el proceso antes de
reemplazar el binario. Pero si la **otra** máquina estaba capturando hacia la que se reinicia, esa otra
se queda con el input hasta que la recuperación lo libere (punto 2).

## Qué hacer si se traba

| Situación | Qué hacer |
|---|---|
| El mouse "se fue" a la otra máquina y no vuelve | **Ctrl + Shift + Win + Alt** (o **Ctrl + Shift + Cmd + Option** en la Mac) |
| La Windows no responde a su propio mouse ni teclado | **Ctrl + Alt + Supr** → Administrador de tareas → finalizar `vkvm.exe` |
| La Mac no responde | Desde otra máquina, `ssh` o el teclado físico: `launchctl bootout gui/$(id -u)/com.virtualkvm.tray` |
| Quedó una tecla "pegada" (todo sale con Ctrl o Shift) | Apretá y soltá esa tecla en la máquina afectada. Si sigue, apagá VirtualKvm desde el ícono |

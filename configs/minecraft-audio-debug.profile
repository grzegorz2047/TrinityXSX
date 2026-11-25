; Temporary WASAPI initialization trace for Minecraft Preview.
[application]
id = minecraft-beta
name = Minecraft Preview (XCL)
executable = C:\XCL\xcl.exe
working_directory = C:\XCL
arguments = run --in-process C:\XCL\games\minecraft-beta
dll_overrides = d3d12,d3d12core,dxgi,d3d11=n
prefix = default
runtime = wine-wow64
architecture = pe64
graphics = dxvk

[display]
desktop = 1920x1080

[input]
mode = xinput

[debug]
winedebug = err+all,+mmdevapi
env = VKD3D_SHADER_CACHE_PATH=C:/XCL/cache/minecraft-beta
env = VKD3D_DEBUG=warn
env = VKD3D_LOG_FILE=C:/XCL/vkd3d-minecraft.log
env = DXVK_LOG_PATH=C:/XCL

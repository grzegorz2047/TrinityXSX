; TrinityXSX catalog inside the existing PS5 Wine runtime.
[application]
id = trinityxsx-library
name = TrinityXSX
executable = C:\XCL\xcl-library.exe
working_directory = C:\XCL
arguments = library C:\XCL\games
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
winedebug = err+all
env = VKD3D_SHADER_CACHE_PATH=C:/XCL/cache/trinityxsx-library
env = VKD3D_DEBUG=warn
env = VKD3D_LOG_FILE=C:/XCL/vkd3d-trinityxsx-library.log
env = DXVK_LOG_PATH=C:/XCL

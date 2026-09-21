#ifndef PayloadDir
  #error PayloadDir must point to the verified offline payload
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif

[Setup]
AppId={{61AF8F6E-9BC6-445F-90D4-8F2D334071E9}
AppName=Parla
AppVersion=0.3.1-speech-cleanup-20260921
AppPublisher=Parla
AppPublisherURL=https://github.com/ILoveRestockMonitors/parla
AppSupportURL=https://github.com/ILoveRestockMonitors/parla
DefaultDirName={localappdata}\Programs\Parla
DefaultGroupName=Parla
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Parla-Setup
Compression=lzma2/fast
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\parla.exe
CloseApplications=no
RestartApplications=no
SetupLogging=yes
VersionInfoVersion=0.3.1.0
InfoBeforeFile={#PayloadDir}\INSTALL.txt

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"
Name: "startmenuicon"; Description: "Create a Start menu shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Excludes: "*.pyc,*.pyo"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autodesktop}\Parla"; Filename: "{app}\parla.exe"; Parameters: "--dashboard"; WorkingDir: "{app}"; Tasks: desktopicon
Name: "{group}\Parla"; Filename: "{app}\parla.exe"; Parameters: "--dashboard"; WorkingDir: "{app}"; Tasks: startmenuicon

[Run]
Filename: "{app}\parla.exe"; Parameters: "--dashboard"; Description: "Open Parla"; Flags: nowait postinstall skipifsilent

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    if not Exec(ExpandConstant('{app}\parla.exe'), '--initialize-bundle',
      ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      RaiseException('Parla could not create its first-run settings. Reinstall Parla.');
    if ResultCode <> 0 then
      RaiseException('Parla could not verify its bundled files or initialize settings. Reinstall Parla.');
  end;
end;

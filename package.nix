{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  name = "gitv";

  src = ./.;
  cargoLock = {
    lockFile = ./Cargo.lock;
    allowBuiltinFetchGit = true;
  };

  meta = {
    description = "Terminal-based viewer for GitHub issues";
    homepage = "https://github.com/JayanAXHF/gitv";
    license = with lib.licenses; [mit unlicense];
  };
})

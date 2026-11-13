cask "hex" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "@URL@"
  name "Agent-Desk"
  desc "Local voice dictation"
  homepage "https://anoma.ly/hex/"

  livecheck do
    url "https://anoma.ly/release/hex/desktop/stable.json"
    strategy :json do |json|
      json.dig("release", "version")
    end
  end

  auto_updates true
  depends_on arch: :arm64
  depends_on macos: :sequoia

  app "Agent-Desk.app"

  uninstall quit: "fheuer.agentdesk"

  zap trash: [
    "~/.config/agent-desk",
    "~/Library/Application Support/agent-desk",
    "~/Library/Caches/fheuer.agentdesk",
    "~/Library/HTTPStorages/fheuer.agentdesk",
    "~/Library/Preferences/fheuer.agentdesk.plist",
    "~/Library/Saved Application State/fheuer.agentdesk.savedState",
  ]
end

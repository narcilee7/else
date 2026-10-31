class Else < Formula
  desc "Managed personal agent runtime — you do what matters, else handles the rest"
  homepage "https://github.com/narcilee7/else"
  version "0.1.0"
  license "MIT"

  # 发布时由 scripts/release.sh 填入真实的 sha256。
  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ARM64_SHA256"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_LINUX_ARM64_SHA256"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_LINUX_X86_64_SHA256"
    end
  end

  def install
    bin.install "else"
  end

  def caveats
    <<~EOS
      else 的 Runtime 需要常驻，Connector 才能按计划同步、通知才能按时送达。

        else onboard    准备 runtime 目录并注册后台服务
        else doctor     检查运行状态
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/else --version")
  end
end

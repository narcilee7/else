class Else < Formula
  desc "Managed personal agent runtime — you do what matters, else handles the rest"
  homepage "https://github.com/narcilee7/else"
  version "0.1.0"
  license "MIT"

  # 发布时由 scripts/update-homebrew-formula.py 填入 release 校验和。
  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-apple-darwin.tar.gz"
      sha256 "e0028540f408b0779ebfa65f512ab5eb2c2b4d0be7e378c2673c4406154798d7"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-apple-darwin.tar.gz"
      sha256 "077e4779f9f8ae3761b96402df334d6b29da42e30c6611638d85aa69b88bb4ed"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "c1ad5120fc79f863196dfb600fe7e1a1af3891afc48fa2ed9fbcceee4e5f3c1e"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "cca1eec9ab7b497b738866007bfa7546db4aa1435525873210ff54628b7f50a8"
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

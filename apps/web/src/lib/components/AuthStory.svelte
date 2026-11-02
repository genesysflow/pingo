<div class="auth-story">
  <a class="auth-brand" href="/"><img src="/pingo.svg" alt="" />Pingo</a>
  <div class="auth-pitch">
    <h1>Chat, voice and screen sharing for small teams.</h1>
    <ul class="auth-specs">
      <li><span>Voice</span>Opus at 48 kHz, stereo Studio mode</li>
      <li><span>Noise</span>DeepFilterNet3 on every microphone</li>
      <li><span>Screen</span>Sharp text up to 4K, or 1080p60 motion</li>
      <li><span>Server</span>One binary you run yourself</li>
    </ul>
  </div>
  <div class="auth-meter" aria-hidden="true">
    {#each Array(28) as _, i}<i
        style:--h={`${18 + Math.abs(Math.sin(i * 1.7) * 62)}%`}
        style:--d={`${(i % 7) * 0.13}s`}
      ></i>{/each}
  </div>
</div>

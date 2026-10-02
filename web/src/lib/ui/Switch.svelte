<script lang="ts">
  // Small on/off track (28×16), or the larger input switch (34×20).
  let { on, large = false }: { on: boolean; large?: boolean } = $props();
</script>

<span class="track" class:on class:large><span class="knob"></span></span>

<style>
  .track {
    position: relative;
    display: inline-block;
    width: 28px;
    height: 16px;
    border-radius: 8px;
    background: var(--border);
    flex-shrink: 0;
    transition: background 0.15s;
  }
  .track.on {
    background: var(--accent);
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #fff;
    transition: left 0.15s;
  }
  .on .knob {
    left: 14px;
  }
  .large {
    width: 34px;
    height: 20px;
    border-radius: 10px;
  }
  .large .knob {
    width: 16px;
    height: 16px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
  }
  .large.on .knob {
    left: 16px;
  }
</style>

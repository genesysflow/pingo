<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" fill="currentColor" aria-hidden="true">
  {#each [0, 45, 90, 135, 180, 225, 270, 315] as angle}<ellipse
      cx="50"
      cy="24"
      rx="7"
      ry="20"
      transform={`rotate(${angle} 50 50)`}
    />{/each}<circle cx="50" cy="50" r="8" />
</svg>

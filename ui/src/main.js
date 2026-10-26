import { mount } from 'svelte';
import '@fontsource-variable/bricolage-grotesque/wght.css';
import '@fontsource-variable/instrument-sans/wght.css';
import './app.css';
import App from './App.svelte';
import Palette from './Palette.svelte';
import Suggest from './Suggest.svelte';
import Panel from './Panel.svelte';

// A trackpad pinch arrives as ctrl+wheel and would zoom the browser's own UI; pages zoom with Cmd +/-.
addEventListener('wheel', (e) => e.ctrlKey && e.preventDefault(), { passive: false });

const view = new URLSearchParams(location.search).get('view');
const Root = view === 'palette' ? Palette : view === 'suggest' ? Suggest : view ? Panel : App;
mount(Root, { target: document.getElementById('app'), props: { view } });
